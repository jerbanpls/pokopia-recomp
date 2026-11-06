// SPDX-License-Identifier: GPL-3.0-or-later
// Frame limiter for the low-latency design: begin_frame() waits for the frame's scheduled start BEFORE the caller samples input,
// so input is read as late as possible and the display queue stays short. The wait is a high-resolution waitable timer for
// the bulk and a spin on the performance counter for the last stretch. Schedules are absolute (next = previous + period), so
// the average rate does not drift for small hiccups; a frame that starts late is made up for by at most 10 % of a period (so
// the next interval is never shorter than 0.9 period: no catch-up bursts) and after a stall of more than 4 periods the
// schedule restarts from now.
#pragma once
#include <cstdint>

namespace nxrt {

class FrameLimiter {
public:
    FrameLimiter();
    ~FrameLimiter();
    FrameLimiter(const FrameLimiter&) = delete;
    FrameLimiter& operator=(const FrameLimiter&) = delete;

    // 0 = unlimited (begin_frame never waits; it still records frame times). Safe to call from any thread.
    void set_target_fps(double fps);
    double target_fps() const;

    // A function that runs at the very start of every begin_frame(), on the thread that calls it (the presenting thread). Play mode uses it to
    // apply present-mode changes at a point where nothing else touches the swapchain. nullptr removes it. Safe to call from any thread.
    using PreFrameFn = void (*)(void* user);
    void set_pre_frame_hook(PreFrameFn fn, void* user);

    // Call at the top of every frame, before polling input. Blocks until the scheduled start, returns the frame start time
    // in nanoseconds of the monotonic clock now_ns().
    uint64_t begin_frame();

    // The same wait without the pre-frame hook, for a limiter that runs on the thread that samples input (the game's main thread) while the hook
    // still has to run on the presenting thread: that thread calls run_pre_frame_hook() itself (S18).
    uint64_t begin_frame_nohook();
    void run_pre_frame_hook();

    // Restart the schedule (after a pause, a minimise, a mode change).
    void reset_schedule();

    struct Stats {
        uint64_t frames = 0;            // frame intervals measured (begin_frame calls minus one)
        double mean_ms = 0;             // mean interval between frame starts
        double min_ms = 0, max_ms = 0;
        double p99_ms = 0;              // 99th percentile of the intervals
        double stddev_ms = 0;
        double fps = 0;                 // 1000 / mean_ms
        // Limiter accuracy, separate from what the frame body costs: frames whose body finished before the scheduled start had to
        // wait ("waited"); for those, how late the start came against the schedule. A frame whose body overran the period
        // ("overran") starts at once and is not the limiter's doing.
        uint64_t waited = 0, overran = 0;
        double overran_fraction = 0;    // overran / (waited + overran)
        double late_p99_ms = 0, late_max_ms = 0;   // lateness of waited frames
        double late_over_half_ms = 0;   // fraction of the waited frames that started more than 0.5 ms late (a robust accuracy measure with few frames, unlike p99)
    };
    Stats stats() const;                // over the intervals since reset_stats() (at most the last 8192)
    void reset_stats();

    // Last wait: how long begin_frame() slept/spun (ns), and how late the frame started against its schedule (ns, >= 0).
    uint64_t last_wait_ns() const;
    uint64_t last_late_ns() const;

    static uint64_t now_ns();           // QueryPerformanceCounter in nanoseconds

private:
    struct Impl;
    Impl* impl_;
};

// For a VRR (G-Sync / FreeSync) display with vsync on: a cap a little under the refresh rate keeps the frame rate inside the
// VRR window so that the display never falls back to the plain vsync queue. refresh - max(3, refresh^2 / 3600) (Blur
// Busters formula, at least 3 under), never below 1. 60 -> 57, 120 -> 116, 144 -> 138.24, 240 -> 224.
double suggest_vrr_cap(double refresh_hz);

// Frame cap snapped to a divisor of the refresh rate (F5): with vsync on, a cap that is not a divisor shows frames for uneven numbers of
// refreshes (40 on 90 Hz: 2, 2, 3, 2, 2, 3 ... refreshes). k = ceil(hz / (cap * 1.005)) (0.5 % tolerance for reported rates such as 79.98),
// cap' = hz / k, never above the requested cap beyond the tolerance. 90 Hz: 40 -> 30, 45 -> 45, 30 -> 30; 80 Hz: 40 -> 40; 60 Hz: 40 -> 30,
// 45 -> 30; 40 Hz: 40 -> 40 (k = 1: no snap). Only for 0 < cap < hz with hz > 0; otherwise (or when k would be 1) k = 0 and cap is returned.
struct FrameSnap {
    double cap = 0;     // the cap to run at (hz / k when snapped, else the requested cap)
    int k = 0;          // refresh intervals per frame when snapped (>= 2), else 0
};
FrameSnap snap_frame_cap(double cap, double hz);

// While the game loads (a black boot screen, a loading screen: a presented frame with almost no draws) no limiter waits: the loading code
// advances once per frame, so pacing it at 60 Hz stretches boot and loads. The NVN replay sets it ([display] unpaced_loading, default on).
void frame_limiter_set_unpaced(bool on);
bool frame_limiter_unpaced();
}  // namespace nxrt
