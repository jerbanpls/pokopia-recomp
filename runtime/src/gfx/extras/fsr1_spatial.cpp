// SPDX-License-Identifier: GPL-3.0-or-later
// FSR 1 spatial upscaler (fsr1_spatial.h).
#include "extras/fsr1_spatial.h"

#include <algorithm>
#include <cstring>
#include <vector>

#include "nxrt/log.h"
#include "extras/fsr1_consts.h"

namespace nxrt::extras {
namespace {
constexpr const char* kTag = "fsr1";

#include "extras/extras_shaders.inc"

struct EasuPush {
    uint32_t con[16];
    float opt[4];
};
struct RcasPush {
    uint32_t con[4];
    float opt[4];
};

void image_barrier(VkCommandBuffer cb, VkImage img, VkImageLayout from, VkImageLayout to, VkPipelineStageFlags2 src_stage, VkAccessFlags2 src_access,
                   VkPipelineStageFlags2 dst_stage, VkAccessFlags2 dst_access) {
    VkImageMemoryBarrier2 b = {VK_STRUCTURE_TYPE_IMAGE_MEMORY_BARRIER_2};
    b.srcStageMask = src_stage;
    b.srcAccessMask = src_access;
    b.dstStageMask = dst_stage;
    b.dstAccessMask = dst_access;
    b.oldLayout = from;
    b.newLayout = to;
    b.srcQueueFamilyIndex = b.dstQueueFamilyIndex = VK_QUEUE_FAMILY_IGNORED;
    b.image = img;
    b.subresourceRange = {VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1};
    VkDependencyInfo di = {VK_STRUCTURE_TYPE_DEPENDENCY_INFO};
    di.imageMemoryBarrierCount = 1;
    di.pImageMemoryBarriers = &b;
    vkCmdPipelineBarrier2(cb, &di);
}

VkShaderModule make_module(VkDevice d, const uint32_t* code, size_t bytes) {
    VkShaderModuleCreateInfo mi = {VK_STRUCTURE_TYPE_SHADER_MODULE_CREATE_INFO};
    mi.codeSize = bytes;
    mi.pCode = code;
    VkShaderModule m = VK_NULL_HANDLE;
    return vkCreateShaderModule(d, &mi, nullptr, &m) == VK_SUCCESS ? m : VK_NULL_HANDLE;
}
}  // namespace

bool Fsr1Spatial::init(gfx::Device& dev) {
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

    auto set_layout = [&](VkDescriptorType first, VkDescriptorSetLayout* out) {
        const VkDescriptorSetLayoutBinding b[2] = {{0, first, 1, VK_SHADER_STAGE_COMPUTE_BIT, nullptr}, {1, VK_DESCRIPTOR_TYPE_STORAGE_IMAGE, 1, VK_SHADER_STAGE_COMPUTE_BIT, nullptr}};
        VkDescriptorSetLayoutCreateInfo li = {VK_STRUCTURE_TYPE_DESCRIPTOR_SET_LAYOUT_CREATE_INFO};
        li.bindingCount = 2;
        li.pBindings = b;
        return vkCreateDescriptorSetLayout(d, &li, nullptr, out) == VK_SUCCESS;
    };
    auto pipe_layout = [&](VkDescriptorSetLayout sl, uint32_t push, VkPipelineLayout* out) {
        VkPushConstantRange pcr = {VK_SHADER_STAGE_COMPUTE_BIT, 0, push};
        VkPipelineLayoutCreateInfo li = {VK_STRUCTURE_TYPE_PIPELINE_LAYOUT_CREATE_INFO};
        li.setLayoutCount = 1;
        li.pSetLayouts = &sl;
        li.pushConstantRangeCount = 1;
        li.pPushConstantRanges = &pcr;
        return vkCreatePipelineLayout(d, &li, nullptr, out) == VK_SUCCESS;
    };
    if (!set_layout(VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER, &easu_set_layout_)) return fail("EASU set layout");
    if (!set_layout(VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE, &rcas_set_layout_)) return fail("RCAS set layout");
    if (!pipe_layout(easu_set_layout_, sizeof(EasuPush), &easu_layout_)) return fail("EASU pipeline layout");
    if (!pipe_layout(rcas_set_layout_, sizeof(RcasPush), &rcas_layout_)) return fail("RCAS pipeline layout");

    const VkDescriptorPoolSize ps[3] = {{VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER, kSetRing}, {VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE, kSetRing}, {VK_DESCRIPTOR_TYPE_STORAGE_IMAGE, 2 * kSetRing}};
    VkDescriptorPoolCreateInfo dpi = {VK_STRUCTURE_TYPE_DESCRIPTOR_POOL_CREATE_INFO};
    dpi.maxSets = 2 * kSetRing;
    dpi.poolSizeCount = 3;
    dpi.pPoolSizes = ps;
    if (vkCreateDescriptorPool(d, &dpi, nullptr, &pool_) != VK_SUCCESS) return fail("descriptor pool");
    auto alloc = [&](VkDescriptorSetLayout l, VkDescriptorSet* out) {
        VkDescriptorSetLayout ls[kSetRing];
        for (auto& x : ls) x = l;
        VkDescriptorSetAllocateInfo ai = {VK_STRUCTURE_TYPE_DESCRIPTOR_SET_ALLOCATE_INFO};
        ai.descriptorPool = pool_;
        ai.descriptorSetCount = kSetRing;
        ai.pSetLayouts = ls;
        return vkAllocateDescriptorSets(d, &ai, out) == VK_SUCCESS;
    };
    if (!alloc(easu_set_layout_, easu_sets_) || !alloc(rcas_set_layout_, rcas_sets_)) return fail("descriptor sets");

    auto compute = [&](const uint32_t* code, size_t bytes, VkPipelineLayout l, VkPipeline* out) {
        VkShaderModule m = make_module(d, code, bytes);
        if (!m) return false;
        VkComputePipelineCreateInfo ci = {VK_STRUCTURE_TYPE_COMPUTE_PIPELINE_CREATE_INFO};
        ci.stage = {VK_STRUCTURE_TYPE_PIPELINE_SHADER_STAGE_CREATE_INFO, nullptr, 0, VK_SHADER_STAGE_COMPUTE_BIT, m, "main", nullptr};
        ci.layout = l;
        const VkResult r = vkCreateComputePipelines(d, VK_NULL_HANDLE, 1, &ci, nullptr, out);
        vkDestroyShaderModule(d, m, nullptr);
        return r == VK_SUCCESS;
    };
    if (!compute(kFsrEasuComp, sizeof kFsrEasuComp, easu_layout_, &easu_)) return fail("EASU pipeline");
    if (!compute(kFsrRcasComp, sizeof kFsrRcasComp, rcas_layout_, &rcas_)) return fail("RCAS pipeline");
    return true;
}

void Fsr1Spatial::free_target(Target& t) {
    if (t.img) dev_->destroy_image(t.img);
    t = Target();
}

bool Fsr1Spatial::ensure_target(Target& t, VkExtent2D e, const char* name) {
    if (t.img && t.extent.width == e.width && t.extent.height == e.height) return true;
    if (t.img) {
        dev_->wait_idle();
        free_target(t);
    }
    gfx::ImageDesc id;
    id.format = VK_FORMAT_R16G16B16A16_SFLOAT;
    id.width = e.width;
    id.height = e.height;
    id.usage = VK_IMAGE_USAGE_STORAGE_BIT | VK_IMAGE_USAGE_SAMPLED_BIT | VK_IMAGE_USAGE_TRANSFER_DST_BIT;
    id.name = name;
    std::string why;
    if (dev_->create_image(id, &t.img, &why) != VK_SUCCESS) {
        NXLOG_E(kTag, "%s: %s", name, why.c_str());
        return false;
    }
    t.extent = e;
    t.initialised = false;
    return true;
}

void Fsr1Spatial::shutdown() {
    if (!dev_) return;
    VkDevice d = dev_->device();
    dev_->wait_idle();
    free_target(easu_img_);
    free_target(rcas_img_);
    if (easu_) vkDestroyPipeline(d, easu_, nullptr);
    if (rcas_) vkDestroyPipeline(d, rcas_, nullptr);
    if (easu_layout_) vkDestroyPipelineLayout(d, easu_layout_, nullptr);
    if (rcas_layout_) vkDestroyPipelineLayout(d, rcas_layout_, nullptr);
    if (pool_) vkDestroyDescriptorPool(d, pool_, nullptr);
    if (easu_set_layout_) vkDestroyDescriptorSetLayout(d, easu_set_layout_, nullptr);
    if (rcas_set_layout_) vkDestroyDescriptorSetLayout(d, rcas_set_layout_, nullptr);
    if (linear_) vkDestroySampler(d, linear_, nullptr);
    easu_ = rcas_ = VK_NULL_HANDLE;
    easu_layout_ = rcas_layout_ = VK_NULL_HANDLE;
    pool_ = VK_NULL_HANDLE;
    easu_set_layout_ = rcas_set_layout_ = VK_NULL_HANDLE;
    linear_ = VK_NULL_HANDLE;
    std::fill(std::begin(easu_sets_), std::end(easu_sets_), VK_NULL_HANDLE);
    std::fill(std::begin(rcas_sets_), std::end(rcas_sets_), VK_NULL_HANDLE);
    next_easu_ = next_rcas_ = 0;
    dev_ = nullptr;
}

VkImageView Fsr1Spatial::upscale(VkCommandBuffer cb, VkImageView src, VkExtent2D in_view, VkExtent2D in_tex, VkExtent2D out, bool linear_hdr, float prescale, float sharpness,
                                 VkExtent2D out_tex) {
    if (!dev_ || !src || !out.width || !out.height || !in_view.width || !in_view.height) return VK_NULL_HANDLE;
    // the result image may be larger than out (both passes write and read only inside out: opt.yz); the EASU intermediate is out
    const VkExtent2D res{std::max(out.width, out_tex.width), std::max(out.height, out_tex.height)};
    if (!ensure_target(easu_img_, out, "fsr easu") || !ensure_target(rcas_img_, res, "fsr rcas")) return VK_NULL_HANDLE;
    VkDevice d = dev_->device();
    const bool fresh = !rcas_img_.initialised;
    for (Target* t : {&easu_img_, &rcas_img_}) {
        image_barrier(cb, t->img.image, t->initialised ? VK_IMAGE_LAYOUT_GENERAL : VK_IMAGE_LAYOUT_UNDEFINED, VK_IMAGE_LAYOUT_GENERAL,
                      VK_PIPELINE_STAGE_2_FRAGMENT_SHADER_BIT | VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT, VK_ACCESS_2_SHADER_READ_BIT | VK_ACCESS_2_SHADER_WRITE_BIT,
                      VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT, VK_ACCESS_2_SHADER_WRITE_BIT | VK_ACCESS_2_SHADER_READ_BIT);
        t->initialised = true;
    }
    if (fresh && (res.width > out.width || res.height > out.height)) {   // a result image larger than out: zero it once (the draw that samples it may touch the texel past out; no NaN from undefined memory)
        const VkClearColorValue zero{};
        const VkImageSubresourceRange all{VK_IMAGE_ASPECT_COLOR_BIT, 0, 1, 0, 1};
        vkCmdClearColorImage(cb, rcas_img_.img.image, VK_IMAGE_LAYOUT_GENERAL, &zero, 1, &all);
        image_barrier(cb, rcas_img_.img.image, VK_IMAGE_LAYOUT_GENERAL, VK_IMAGE_LAYOUT_GENERAL, VK_PIPELINE_STAGE_2_CLEAR_BIT, VK_ACCESS_2_TRANSFER_WRITE_BIT,
                      VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT, VK_ACCESS_2_SHADER_WRITE_BIT | VK_ACCESS_2_SHADER_READ_BIT);
    }
    const float perceptual = linear_hdr ? 1.0f : 0.0f;
    {   // EASU
        EasuPush pc = {};
        fsr1_easu_consts(pc.con, pc.con + 4, pc.con + 8, pc.con + 12, static_cast<float>(in_view.width), static_cast<float>(in_view.height), static_cast<float>(in_tex.width),
                         static_cast<float>(in_tex.height), static_cast<float>(out.width), static_cast<float>(out.height));
        pc.opt[0] = perceptual;
        pc.opt[1] = static_cast<float>(out.width);
        pc.opt[2] = static_cast<float>(out.height);
        pc.opt[3] = prescale;
        VkDescriptorSet set = easu_sets_[next_easu_];
        next_easu_ = (next_easu_ + 1) % kSetRing;
        VkDescriptorImageInfo si = {linear_, src, VK_IMAGE_LAYOUT_SHADER_READ_ONLY_OPTIMAL};
        VkDescriptorImageInfo di = {VK_NULL_HANDLE, easu_img_.img.view, VK_IMAGE_LAYOUT_GENERAL};
        VkWriteDescriptorSet w[2] = {{VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET}, {VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET}};
        w[0].dstSet = set; w[0].dstBinding = 0; w[0].descriptorCount = 1; w[0].descriptorType = VK_DESCRIPTOR_TYPE_COMBINED_IMAGE_SAMPLER; w[0].pImageInfo = &si;
        w[1].dstSet = set; w[1].dstBinding = 1; w[1].descriptorCount = 1; w[1].descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_IMAGE; w[1].pImageInfo = &di;
        vkUpdateDescriptorSets(d, 2, w, 0, nullptr);
        vkCmdBindPipeline(cb, VK_PIPELINE_BIND_POINT_COMPUTE, easu_);
        vkCmdBindDescriptorSets(cb, VK_PIPELINE_BIND_POINT_COMPUTE, easu_layout_, 0, 1, &set, 0, nullptr);
        vkCmdPushConstants(cb, easu_layout_, VK_SHADER_STAGE_COMPUTE_BIT, 0, sizeof pc, &pc);
        vkCmdDispatch(cb, (out.width + 15) / 16, (out.height + 15) / 16, 1);
    }
    image_barrier(cb, easu_img_.img.image, VK_IMAGE_LAYOUT_GENERAL, VK_IMAGE_LAYOUT_GENERAL, VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT, VK_ACCESS_2_SHADER_WRITE_BIT,
                  VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT, VK_ACCESS_2_SHADER_READ_BIT);
    {   // RCAS
        RcasPush pc = {};
        fsr1_rcas_consts(pc.con, (1.0f - std::clamp(sharpness, 0.0f, 1.0f)) * 2.0f);
        pc.opt[0] = perceptual;
        pc.opt[1] = static_cast<float>(out.width);
        pc.opt[2] = static_cast<float>(out.height);
        pc.opt[3] = prescale;
        VkDescriptorSet set = rcas_sets_[next_rcas_];
        next_rcas_ = (next_rcas_ + 1) % kSetRing;
        VkDescriptorImageInfo si = {VK_NULL_HANDLE, easu_img_.img.view, VK_IMAGE_LAYOUT_GENERAL};
        VkDescriptorImageInfo di = {VK_NULL_HANDLE, rcas_img_.img.view, VK_IMAGE_LAYOUT_GENERAL};
        VkWriteDescriptorSet w[2] = {{VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET}, {VK_STRUCTURE_TYPE_WRITE_DESCRIPTOR_SET}};
        w[0].dstSet = set; w[0].dstBinding = 0; w[0].descriptorCount = 1; w[0].descriptorType = VK_DESCRIPTOR_TYPE_SAMPLED_IMAGE; w[0].pImageInfo = &si;
        w[1].dstSet = set; w[1].dstBinding = 1; w[1].descriptorCount = 1; w[1].descriptorType = VK_DESCRIPTOR_TYPE_STORAGE_IMAGE; w[1].pImageInfo = &di;
        vkUpdateDescriptorSets(d, 2, w, 0, nullptr);
        vkCmdBindPipeline(cb, VK_PIPELINE_BIND_POINT_COMPUTE, rcas_);
        vkCmdBindDescriptorSets(cb, VK_PIPELINE_BIND_POINT_COMPUTE, rcas_layout_, 0, 1, &set, 0, nullptr);
        vkCmdPushConstants(cb, rcas_layout_, VK_SHADER_STAGE_COMPUTE_BIT, 0, sizeof pc, &pc);
        vkCmdDispatch(cb, (out.width + 15) / 16, (out.height + 15) / 16, 1);
    }
    image_barrier(cb, rcas_img_.img.image, VK_IMAGE_LAYOUT_GENERAL, VK_IMAGE_LAYOUT_GENERAL, VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT, VK_ACCESS_2_SHADER_WRITE_BIT,
                  VK_PIPELINE_STAGE_2_FRAGMENT_SHADER_BIT | VK_PIPELINE_STAGE_2_COMPUTE_SHADER_BIT, VK_ACCESS_2_SHADER_READ_BIT);
    ++frames_;
    return rcas_img_.img.view;
}

}  // namespace nxrt::extras
