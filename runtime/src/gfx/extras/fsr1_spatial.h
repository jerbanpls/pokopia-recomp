// SPDX-License-Identifier: GPL-3.0-or-later
// AMD FSR 1 spatial upscaler (S14): EASU then RCAS as two compute passes into float16 images the size of the output. Used by the output pass
// (the finished picture) and by the scene upscaler (the HDR scene before the game's tone-map). The shaders are in extras_shaders.inc, the constants come from
// fsr1_consts. In linear HDR mode the filters run on c = x*k / (1 + x*k) so values above 1 survive them.
#pragma once
#include <volk.h>

#include <cstdint>

#include "nxrt/gfx.h"
#include "extras/spatial_upscaler.h"

namespace nxrt::extras {

class Fsr1Spatial : public SpatialUpscaler {
public:
    Fsr1Spatial() = default;
    ~Fsr1Spatial() override { shutdown(); }
    Fsr1Spatial(const Fsr1Spatial&) = delete;
    Fsr1Spatial& operator=(const Fsr1Spatial&) = delete;

    bool init(gfx::Device& dev);
    void shutdown();
    bool initialised() const { return dev_ != nullptr; }

    // Records into cb (outside any render pass). src is a view of an image in SHADER_READ_ONLY_OPTIMAL; in_view is the part of it to upscale (from texel 0, 0), in_tex
    // the size of the whole image; out the size of the result. linear_hdr: the source is scene-linear float and may exceed 1 (prescale k scales it before the
    // compression; 1 for display-referred values, about 0.1 for the game's pre-exposure scene colour); false: the source is already in [0,1] (sRGB-style).
    // sharpness 0..1 (RCAS; 1 = sharpest). Returns the view of the result, an image in GENERAL layout with a barrier recorded that makes it readable by compute and
    // fragment shaders; the view stays valid until the next call that changes `out`. Null on failure.
    VkImageView upscale(VkCommandBuffer cb, VkImageView src, VkExtent2D in_view, VkExtent2D in_tex, VkExtent2D out, bool linear_hdr, float prescale, float sharpness,
                        VkExtent2D out_tex = {}) override;
    uint32_t frames() const { return frames_; }

    static constexpr uint32_t kSetRing = 16;

private:
    struct Target {
        gfx::Image img;
        VkExtent2D extent = {};
        bool initialised = false;
    };
    bool ensure_target(Target& t, VkExtent2D e, const char* name);
    void free_target(Target& t);

    gfx::Device* dev_ = nullptr;
    VkSampler linear_ = VK_NULL_HANDLE;
    VkDescriptorSetLayout easu_set_layout_ = VK_NULL_HANDLE, rcas_set_layout_ = VK_NULL_HANDLE;
    VkPipelineLayout easu_layout_ = VK_NULL_HANDLE, rcas_layout_ = VK_NULL_HANDLE;
    VkPipeline easu_ = VK_NULL_HANDLE, rcas_ = VK_NULL_HANDLE;
    VkDescriptorPool pool_ = VK_NULL_HANDLE;
    VkDescriptorSet easu_sets_[kSetRing] = {}, rcas_sets_[kSetRing] = {};
    uint32_t next_easu_ = 0, next_rcas_ = 0;
    Target easu_img_, rcas_img_;
    uint32_t frames_ = 0;
};

}  // namespace nxrt::extras
