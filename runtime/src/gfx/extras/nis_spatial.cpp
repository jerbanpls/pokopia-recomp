// SPDX-License-Identifier: GPL-3.0-or-later
// NVIDIA Image Scaling spatial upscaler (nis_spatial.h).
#include "extras/nis_spatial.h"

#include <algorithm>
#include <cstddef>
#include <cstring>
#include <iterator>
#include <string>

#include "nxrt/log.h"
#include "../../../third_party/nvidia_nis/NIS_Config.h"

namespace nxrt::extras {
namespace {
constexpr const char* kTag = "nis";
constexpr uint32_t kBlockW = 32, kBlockH = 24;   // NIS_BLOCK_WIDTH / NIS_BLOCK_HEIGHT of the generated shader
constexpr uint32_t kPushBytes = 112;             // NISConfig up to and including reserved1

#include "extras/nis_shaders.inc"

void to_general(VkCommandBuffer cb, VkImage img, VkImageLayout from, VkPipelineStageFlags2 src_stage, VkAccessFlags2 src_access, VkPipelineStageFlags2 dst_stage,
                VkAccessFlags2 dst_access) {
    VkImageMemoryBarrier2 b = {VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER_2};
    b.srcStageMask = src_stage;
    b.srcAccessMask = src_access;
    b.dstStageMask = dst_stage;
    b.dstAccessMask = dst_access;
    b.oldLayout = from;
    b.newLayout = VK_IMAGE_LAYOUT_GENERAL;
    b.srcQueueFamilyIndex = b.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
    b.image = img;
    b.subresourceRange = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1};
    VkDependencyInfo di = {VK_STRUCTURE_TYPE_DEPENDENCY_INFO};
    di.imageMemoryBarrierCount = 1;
    di.pImageMemoryBarriers = &b;
    vkCmdPipelineBarrier2(cb, &di);
}
}  // namespace

static_assert(offsetof(NISConfig, reserved1) + sizeof(float) == kPushBytes, "NISConfig layout no longer matches the shader's push constants");

bool NisSpatial::init(gfx::Device& dev) {
    if (dev_) return true;
    dev_ = &dev;
    VkDevice d = dev.device();
    auto fail = [&](const char* what) {
        NXLOG_E(kTag, "%s failed", what);
        shutdown();
        return false;
    };
    VkSamplerCreateInfo sci = {VK_STRUCTURE_TYPE_SAMPLER_CREATE_INFO};
    sci.magFilter = sci.minFilter = VK_FILTER_LINEAR;
    sci.mipmapMode = VK_SAMPLER_MIPMAP_MODE_NEAREST;
    sci.addressModeU = sci.addressModeV = sci.addressModeW = VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE;
    sci.maxLod = 0.0f;
    if (vkCreateSampler(d, &sci, nullptr, &linear_) != VK_SUCCESS) return fail("sampler");

    const VkDescriptorSetLayoutBinding b[3] = {{0, VK_DESCRIPTOR_TYPE_SAMPLER, 1, VK_SHADER_STAGE_COMPUTE_BIT, nullptr},
                                               {1, VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE, 1, VK_SHADER_STAGE_COMPUTE_BIT, nullptr},
                                               {2, VK_DESCRIPTOR_TYPE_STORAGE_IMAGE, 1, VK_SHADER_STAGE_COMPUTE_BIT, nullptr}};
    VkDescriptorSetLayoutCreateInfo sli = {VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO};
    sli.bindingCount = 3;
    sli.pBindings = b;
    if (vkCreateDescriptorSetLayout(d, &sli, nullptr, &set_layout_) != VK_SUCCESS) return fail("set layout");
    VkPushConstantRange pcr = {VK_SHADER_STAGE_COMPUTE_BIT, 0, kPushBytes};
    VkPipelineLayoutCreateInfo pli = {VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO};
    pli.setLayoutCount = 1;
    pli.pSetLayouts = &set_layout_;
    pli.pushConstantRangeCount = 1;
    pli.pPushConstantRanges = &pcr;
    if (vkCreatePipelineLayout(d, &pli, nullptr, &layout_) != VK_SUCCESS) return fail("pipeline layout");

    const VkDescriptorPoolSize ps[3] = {{VK_DESCRIPTOR_TYPE_SAMPLER, kSetRing}, {VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE, kSetRing}, {VK_DESCRIPTOR_TYPE_STORAGE_IMAGE, kSetRing}};
    VkDescriptorPoolCreateInfo dpi = {VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO};
    dpi.maxSets = kSetRing;
    dpi.poolSizeCount = 3;
    dpi.pPoolSizes = ps;
    if (vkCreateDescriptorPool(d, &dpi, nullptr, &pool_) != VK_SUCCESS) return fail("descriptor pool");
    VkDescriptorSetLayout ls[kSetRing];
    for (auto& x : ls) x = set_layout_;
    VkDescriptorSetAllocateInfo ai = {VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO};
    ai.descriptorPool = pool_;
    ai.descriptorSetCount = kSetRing;
    ai.pSetLayouts = ls;
    if (vkAllocateDescriptorSets(d, &ai, sets_) != VK_SUCCESS) return fail("descriptor sets");

    VkShaderModuleCreateInfo mi = {VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO};
    mi.codeSize = sizeof kNisScalerComp;
    mi.pCode = kNisScalerComp;
    VkShaderModule m = VK_NULL_HANDLE;
    if (vkCreateShaderModule(d, &mi, nullptr, &m) != VK_SUCCESS) return fail("shader module");
    VkComputePipelineCreateInfo ci = {VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO};
    ci.stage = {VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO, nullptr, 0, VK_SHADER_STAGE_COMPUTE_BIT, m, "main", nullptr};
    ci.layout = layout_;
    const VkResult r = vkCreateComputePipelines(d, VK_NULL_HANDLE, 1, &ci, nullptr, &pipe_);
    vkDestroyShaderModule(d, m, nullptr);
    if (r != VK_SUCCESS) return fail("pipeline");
    NXLOG_I(kTag, "NVIDIA Image Scaling v1.0.3 ready");
    return true;
}

void NisSpatial::shutdown() {
    if (!dev_) return;
    VkDevice d = dev_->device();
    dev_->wait_idle();
    if (img_) dev_->destroy_image(img_);
    img_ = gfx::Image();
    extent_ = {};
    img_init_ = false;
    if (pipe_) vkDestroyPipeline(d, pipe_, nullptr);
    if (layout_) vkDestroyPipelineLayout(d, layout_, nullptr);
    if (pool_) vkDestroyDescriptorPool(d, pool_, nullptr);
    if (set_layout_) vkDestroyDescriptorSetLayout(d, set_layout_, nullptr);
    if (linear_) vkDestroySampler(d, linear_, nullptr);
    pipe_ = VK_NULL_HANDLE;
    layout_ = VK_NULL_HANDLE;
    pool_ = VK_NULL_HANDLE;
    set_layout_ = VK_NULL_HANDLE;
    linear_ = VK_NULL_HANDLE;
    std::fill(std::begin(sets_), std::end(sets_), VK_NULL_HANDLE);
    next_ = 0;
    dev_ = nullptr;
}

bool NisSpatial::supports(VkExtent2D in_view, VkExtent2D out) const {
    if (!in_view.width || !in_view.height || !out.width || !out.height) return false;
    const float sx = static_cast<float>(in_view.width) / static_cast<float>(out.width);
    const float sy = static_cast<float>(in_view.height) / static_cast<float>(out.height);
    return sx >= 0.5f && sx <= 1.0f && sy >= 0.5f && sy <= 1.0f;
}

VkImageView NisSpatial::upscale(VkCommandBuffer cb, VkImageView src, VkExtent2D in_view, VkExtent2D in_tex, VkExtent2D out, bool linear_hdr, float prescale,
                                float sharpness, VkExtent2D out_tex) {
    if (!dev_ || !src || !supports(in_view, out)) return VK_NULL_HANDLE;
    const VkExtent2D res{std::max(out.width, out_tex.width), std::max(out.height, out_tex.height)};   // the image; the scaler writes the out viewport only
    NISConfig cfg = {};
    if (!NVScalerUpdateConfig(cfg, std::clamp(sharpness, 0.0f, 1.0f), 0, 0, in_view.width, in_view.height, in_tex.width, in_tex.height, 0, 0, out.width,
                              out.height, res.width, res.height, NISHDRMode::None))
        return VK_NULL_HANDLE;
    cfg.reserved0 = linear_hdr ? (prescale > 0.0f ? prescale : 1.0f) : 0.0f;   // the shader's kPrescale (0: [0,1] input used as is)
    if (!img_ || extent_.width != res.width || extent_.height != res.height) {
        if (img_) {
            dev_->wait_idle();
            dev_->destroy_image(img_);
            img_ = gfx::Image();
        }
        gfx::ImageDesc id;
        id.format = VK_FORMAT_R16G16B16A16_SFLOAT;
        id.width = res.width;
        id.height = res.height;
        id.usage = VK_IMAGE_USAGE_STORAGE_BIT | VK_IMAGE_USAGE_SAMPLED_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT;
        id.name = "nis out";
        std::string why;
        if (dev_->create_image(id, &img_, &why) != VK_SUCCESS) {
            NXLOG_E(kTag, "output image: %s", why.c_str());
            img_ = gfx::Image();
            return VK_NULL_HANDLE;
        }
        extent_ = res;
        img_init_ = false;
    }
    const bool fresh = !img_init_;
    to_general(cb, img_.image, img_init_ ? VK_IMAGE_LAYOUT_GENERAL : VK_IMAGE_LAYOUT_UNDEFINED,
               VK_PIPELINE_STAGE_2_FRAGMENT_SHADER_BIT | VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT, VK_ACCESS_2_SHADER_READ_BIT, VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT,
               VK_ACCESS_2_SHADER_WRITE_BIT);
    img_init_ = true;
    if (fresh && (res.width > out.width || res.height > out.height)) {   // a result image larger than out: zero it once (the draw that samples it may touch the texel past out; no NaN from undefined memory)
        const VkClearColorValue zero{};
        const VkImageSubresourceRange all{VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1};
        vkCmdClearColorImage(cb, img_.image, VK_IMAGE_LAYOUT_GENERAL, &zero, 1, &all);
        to_general(cb, img_.image, VK_IMAGE_LAYOUT_GENERAL, VK_PIPELINE_STAGE_2_CLEAR_BIT, VK_ACCESS_2_TRANSFER_WRITE_BIT, VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT,
                   VK_ACCESS_2_SHADER_WRITE_BIT);
    }

    VkDescriptorSet set = sets_[next_];
    next_ = (next_ + 1) % kSetRing;
    const VkDescriptorImageInfo smp = {linear_, VK_NULL_HANDLE, VK_IMAGE_LAYOUT_UNDEFINED};
    const VkDescriptorImageInfo in = {VK_NULL_HANDLE, src, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL};
    const VkDescriptorImageInfo dst = {VK_NULL_HANDLE, img_.view, VK_IMAGE_LAYOUT_GENERAL};
    const VkDescriptorType types[3] = {VK_DESCRIPTOR_TYPE_SAMPLER, VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE, VK_DESCRIPTOR_TYPE_STORAGE_IMAGE};
    const VkDescriptorImageInfo* infos[3] = {&smp, &in, &dst};
    VkWriteDescriptorSet w[3] = {};
    for (uint32_t i = 0; i < 3; ++i) {
        w[i].sType = VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET;
        w[i].dstSet = set;
        w[i].dstBinding = i;
        w[i].descriptorCount = 1;
        w[i].descriptorType = types[i];
        w[i].pImageInfo = infos[i];
    }
    vkUpdateDescriptorSets(dev_->device(), 3, w, 0, nullptr);
    uint8_t push[kPushBytes];
    std::memcpy(push, &cfg, kPushBytes);
    vkCmdBindPipeline(cb, VK_PIPELINE_BIND_POINT_COMPUTE, pipe_);
    vkCmdBindDescriptorSets(cb, VK_PIPELINE_BIND_POINT_COMPUTE, layout_, 0, 1, &set, 0, nullptr);
    vkCmdPushConstants(cb, layout_, VK_SHADER_STAGE_COMPUTE_BIT, 0, kPushBytes, push);
    vkCmdDispatch(cb, (out.width + kBlockW - 1) / kBlockW, (out.height + kBlockH - 1) / kBlockH, 1);
    to_general(cb, img_.image, VK_IMAGE_LAYOUT_GENERAL, VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT, VK_ACCESS_2_SHADER_WRITE_BIT,
               VK_PIPELINE_STAGE_2_FRAGMENT_SHADER_BIT | VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT, VK_ACCESS_2_SHADER_READ_BIT);
    ++frames_;
    return img_.view;
}

}  // namespace nxrt::extras
