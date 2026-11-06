// SPDX-License-Identifier: GPL-3.0-or-later
// NVIDIA Image Scaling (NIS v1.0.3, MIT) spatial upscaler: one compute pass (scale + sharpen) into a float16 image the size of the output.
// An alternative to FSR 1 for the output pass and the scene upscaler; it runs on any GPU. NIS scales 1x to 2x per side; the callers fall back to
// FSR 1 beyond that. The shader is in nis_shaders.inc (scripts/gen_nis_shaders.py), the constants come from the SDK's NVScalerUpdateConfig.
#pragma once
#include <volk.h>

#include <cstdint>

#include "nxrt/gfx.h"
#include "extras/spatial_upscaler.h"

namespace nxrt::extras {

class NisSpatial : public SpatialUpscaler {
public:
    NisSpatial() = default;
    ~NisSpatial() override { shutdown(); }
    NisSpatial(const NisSpatial&) = delete;
    NisSpatial& operator=(const NisSpatial&) = delete;

    bool init(gfx::Device& dev);
    void shutdown();
    bool initialised() const { return dev_ != nullptr; }

    VkImageView upscale(VkCommandBuffer cb, VkImageView src, VkExtent2D in_view, VkExtent2D in_tex, VkExtent2D out, bool linear_hdr, float prescale,
                        float sharpness, VkExtent2D out_tex = {}) override;
    bool supports(VkExtent2D in_view, VkExtent2D out) const override;
    uint32_t frames() const { return frames_; }

    static constexpr uint32_t kSetRing = 16;

private:
    gfx::Device* dev_ = nullptr;
    VkSampler linear_ = VK_NULL_HANDLE;
    VkDescriptorSetLayout set_layout_ = VK_NULL_HANDLE;
    VkPipelineLayout layout_ = VK_NULL_HANDLE;
    VkPipeline pipe_ = VK_NULL_HANDLE;
    VkDescriptorPool pool_ = VK_NULL_HANDLE;
    VkDescriptorSet sets_[kSetRing] = {};
    uint32_t next_ = 0;
    gfx::Image img_;
    VkExtent2D extent_ = {};
    bool img_init_ = false;
    uint32_t frames_ = 0;
};

}  // namespace nxrt::extras
