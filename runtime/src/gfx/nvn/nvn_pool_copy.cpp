// SPDX-License-Identifier: GPL-3.0-or-later
// [nvn] pool_copy = auto (default: on under Wine) | on | off: memory pools as copies (shadows) instead of imports, phase 1 (no write
// tracking). Under Proton every vkQueueSubmit costs more the more userptr memory is imported (amdgpu revalidates every userptr BO of the
// submission's BO list): with whole pools imported (3.8 GiB) the game froze inside vkQueueSubmit, and the lazy pool's ~2 GiB of live imports
// still cost ~17.8 ms per batch submission on the Deck. Here a pool keeps the lazy pool's cell structure (nvn_gpu_pool.cpp: cells, ext,
// cell_ext, spans, idle drop), but each cell holds a host-visible Vulkan buffer of its own (MemoryUse::Shadow) instead of an import of the
// guest pages, so no submission carries a userptr BO.
//
// The rule that keeps the shadow right without knowing what the CPU writes:
//  * touch (resolve_in_pool, render thread): every resolve copies the page-rounded range it binds from guest memory into the shadow, unless
//    that page was copied already in the submission being recorded (page_copied) or a GPU write into it is still in flight (page_gpu above
//    the completed serial: the shadow holds newer bytes than guest memory then). A bind of unknown size touches to the end of its chunk.
//  * record (note_gpu_write, render thread): every range a command may write in pool memory is recorded with the submission's serial
//    (LandRec), and its pages' page_gpu set to that serial.
//  * land (read_completed, any thread; and the nvn-pool-land thread): once the timeline reaches a record's serial, the recorded range is
//    copied from the shadow back into guest memory, before that serial is published as complete (complete_serial). So a game thread that
//    waits for a sync, polls one, or reads an event word the GPU wrote, finds the GPU's bytes in its memory, as with an import.
// Known phase-1 limits: a CPU write to bytes of a range the GPU may write, made while that submission is in flight, is overwritten at
// landing (the whole recorded range lands, not only what a shader stored: landed_kb); a GPU write through a span and a read of the same bytes
// through another chunk (or the reverse) while that write is in flight sees the other buffer's bytes (span_writes counts such writes).
// Design after totk-pc-recomp runtime/vk_memory.c (shadow storage) and ultratears src/gfx/backend_vk.c (pool copies); the code is new.
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <windows.h>

#include <algorithm>
#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstring>
#include <deque>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

#include "nxrt/config.h"
#include "nxrt/ftrace.h"
#include "nxrt/gauge.h"
#include "nxrt/log.h"
#include "nxrt/write_watch.h"
#include "nvn_gpu_internal.h"

namespace nxrt::nvn::be::gpu {
namespace {

constexpr const char* kTag = "nvn.pool";
constexpr uint64_t kPage = 4096;
constexpr uint64_t kOutside = ~0ull;      // page_copied of a copy made outside a recording: equal to no serial, and not "never" (0)
constexpr uint64_t kNone = ~0ull;         // g_land_min with no record

bool wine() {
    static const bool w = [] {
        HMODULE nt = GetModuleHandleW(L"ntdll.dll");
        return nt && GetProcAddress(nt, "wine_get_version");
    }();
    return w;
}

struct Cfg {
    bool on = false;
    uint64_t cell = 16ull << 20;
    uint32_t idle = 600;
    uint64_t cap = 1536ull << 20;
};

const Cfg& cfg() {
    static const Cfg c = [] {
        Cfg r;
        const std::string v = config_get_str("nvn", "pool_copy", "auto");
        r.on = v == "auto" ? wine() : (v == "on" || v == "1" || v == "true");
        r.cell = static_cast<uint64_t>(std::clamp<int64_t>(config_get_int("nvn", "pool_copy_cell_mb", 16), 1, 1024)) << 20;
        r.idle = static_cast<uint32_t>(std::clamp<int64_t>(config_get_int("nvn", "pool_copy_idle", 600), 0, 1 << 30));
        r.cap = static_cast<uint64_t>(std::max<int64_t>(0, config_get_int("nvn", "pool_copy_mb", 1536))) << 20;
        NXLOG_I(kTag, "pool_copy %s (%s%s)", r.on ? "on" : "off", v.c_str(), v == "auto" ? (wine() ? ", Wine" : ", not Wine") : "");
        if (r.on)
            NXLOG_I(kTag, "pool_copy: shadows in cells of %llu MiB, dropped after %u idle presents, cap %llu MiB; pool_lazy, pool_lazy_chunk_mb, "
                          "pool_lazy_idle, pool_lazy_cpu_untile and compute_out_lazy are ignored",
                    static_cast<unsigned long long>(r.cell >> 20), r.idle, static_cast<unsigned long long>(r.cap >> 20));
        return r;
    }();
    return c;
}

// statistics
std::atomic<bool> g_any{false};
std::atomic<int64_t> g_live{0};
std::atomic<uint64_t> g_chunks{0}, g_allocs{0}, g_copied{0}, g_landed{0}, g_records{0}, g_unmatched{0}, g_fallback{0}, g_unreadable{0},
    g_span_writes{0}, g_evicted{0}, g_spans{0};
uint64_t g_sub_serial = 0, g_sub_bytes = 0;     // render thread: bytes copied in by the submission being recorded
std::atomic<int64_t> g_last_sub_kb{0};          // ... and by the last one (gauge)

// GPU writes waiting for their serial, in serial order (recorded by the render thread as it records). Lock order:
// g_live_mu -> g_reg_mu -> g_landing_mu -> g_land_mu. g_land_mu guards only the queue, land_refs and g_land_min and is held for short sections
// (the render thread takes it per recorded write and in pcopy_busy); it is a leaf. g_landing_mu serialises landers and is held across a whole
// landing (pop, memcpy, before_os_write, page bits); before_os_write could call the compute-out guard handler, which copy pools never arm.
struct LandRec {
    PoolGpu* pool;
    PoolChunk* chunk;
    uint64_t pool_off, size, serial;
};
std::mutex g_land_mu;
std::mutex g_landing_mu;
std::deque<LandRec> g_land;
std::atomic<uint64_t> g_land_min{kNone};        // serial of g_land.front() (kNone: empty), the lock-free test of pcopy_land
thread_local bool t_landing = false;            // a landing on this thread (a nested read_completed returns at once)

// The last resolve of the render thread: the chunk a following gpu_wrote of the same range writes, and pool_copy_touch's chunk.
struct LastResolve {
    PoolGpu* pool = nullptr;
    PoolChunk* chunk = nullptr;
    uint64_t gen = 0;      // g_free_gen when set
};
thread_local LastResolve t_last;
// bumped by pcopy_free: a t_last set before it may point into a chunk another thread destroyed, and is ignored
std::atomic<uint64_t> g_free_gen{0};
inline bool last_valid() { return t_last.chunk && t_last.gen == g_free_gen.load(std::memory_order_acquire); }

// live shadows, for the cap's least-recently-used eviction
std::mutex g_reg_mu;
std::vector<PoolChunk*> g_reg;

// the landing thread
std::mutex g_th_mu;
std::condition_variable g_th_cv;
std::thread g_th;
bool g_th_stop = false;

struct CopyGauges {
    CopyGauges() {
        nxrt::gauge_register("nvn.pool.copy.live_mb", [] { return g_live.load(std::memory_order_relaxed) >> 20; });
        nxrt::gauge_register("nvn.pool.copy.copied_kb", [] { return g_last_sub_kb.load(std::memory_order_relaxed); });
        nxrt::gauge_register("nvn.pool.copy.landed_kb", [] { return static_cast<int64_t>(g_landed.load(std::memory_order_relaxed) >> 10); });
        nxrt::gauge_register("nvn.pool.copy.write_unmatched", [] { return static_cast<int64_t>(g_unmatched.load(std::memory_order_relaxed)); });
        nxrt::gauge_register("nvn.pool.copy.import_fallback", [] { return static_cast<int64_t>(g_fallback.load(std::memory_order_relaxed)); });
        nxrt::gauge_register("nvn.pool.copy.unreadable", [] { return static_cast<int64_t>(g_unreadable.load(std::memory_order_relaxed)); });
        nxrt::gauge_register("nvn.pool.copy.span_writes", [] { return static_cast<int64_t>(g_span_writes.load(std::memory_order_relaxed)); });
        nxrt::hitch_add_counter("pool_copy_copied_kb", []() -> uint64_t { return g_copied.load(std::memory_order_relaxed) >> 10; });
        nxrt::hitch_add_counter("pool_copy_landed_kb", []() -> uint64_t { return g_landed.load(std::memory_order_relaxed) >> 10; });
    }
    ~CopyGauges() {
        for (const char* n : {"nvn.pool.copy.live_mb", "nvn.pool.copy.copied_kb", "nvn.pool.copy.landed_kb", "nvn.pool.copy.write_unmatched",
                              "nvn.pool.copy.import_fallback", "nvn.pool.copy.unreadable", "nvn.pool.copy.span_writes"})
            nxrt::gauge_unregister(n);
    }
};
CopyGauges g_copy_gauges;

// memcpy that survives an access violation (a guest page decommitted under it). No C++ objects here (SEH).
bool seh_copy(void* d, const void* s, size_t n) {
    __try {
        std::memcpy(d, s, n);
    } __except (GetExceptionCode() == EXCEPTION_ACCESS_VIOLATION ? EXCEPTION_EXECUTE_HANDLER : EXCEPTION_CONTINUE_SEARCH) {
        return false;
    }
    return true;
}

// [d, d + n) <- [s, s + n); on a fault, page by page of `s` (in) or `d` (out): a part that cannot be copied is zero-filled when it is the
// shadow side (in) or left alone (out). Returns the parts that could not be copied.
uint64_t guarded_copy(uint8_t* d, const uint8_t* s, uint64_t n, bool in) {
    if (seh_copy(d, s, static_cast<size_t>(n))) return 0;
    uint64_t bad = 0;
    const uint64_t a = reinterpret_cast<uint64_t>(in ? s : d);
    for (uint64_t o = 0; o < n;) {
        const uint64_t k = std::min<uint64_t>(n - o, kPage - ((a + o) & (kPage - 1)));
        if (!seh_copy(d + o, s + o, static_cast<size_t>(k))) {
            if (in) std::memset(d + o, 0, static_cast<size_t>(k));
            ++bad;
        }
        o += k;
    }
    return bad;
}

void land_min_update() { g_land_min.store(g_land.empty() ? kNone : g_land.front().serial, std::memory_order_release); }

void drop_all_records() {
    std::lock_guard<std::mutex> lk(g_land_mu);
    for (const LandRec& r : g_land)
        if (r.chunk->land_refs) --r.chunk->land_refs;
    g_land.clear();
    land_min_update();
}

void land_loop() {
    log_set_thread_name("nvn-pool-land");
    std::unique_lock<std::mutex> lk(g_th_mu);
    while (!g_th_stop) {
        const uint64_t s = g_land_min.load(std::memory_order_acquire);
        if (s == kNone) {
            g_th_cv.wait_for(lk, std::chrono::milliseconds(50));
            continue;
        }
        lk.unlock();
        // A word the game polls in memory with no API call (an event the GPU signals, a report) lands within a few ms of its serial.
        if (timeline_wait(s, 5ull * 1000 * 1000)) {
            poll_completed();
        } else if (device().device_lost()) {   // nothing lands after a device loss: the records go
            drop_all_records();
            std::this_thread::sleep_for(std::chrono::milliseconds(5));
        } else {
            std::this_thread::sleep_for(std::chrono::milliseconds(1));   // (a timeout took 5 ms; an error returns at once)
        }
        lk.lock();
    }
}

void start_lander() {
    std::lock_guard<std::mutex> lk(g_th_mu);
    if (g_th.joinable()) return;
    g_th_stop = false;
    g_th = std::thread(land_loop);
}

void reg_remove(PoolChunk* c) {
    std::lock_guard<std::mutex> lk(g_reg_mu);
    g_reg.erase(std::remove(g_reg.begin(), g_reg.end(), c), g_reg.end());
}

VkBufferUsageFlags shadow_usage() {      // what import_range gives an import's buffer
    return VK_BUFFER_USAGE_VERTEX_BUFFER_BIT | VK_BUFFER_USAGE_INDEX_BUFFER_BIT | VK_BUFFER_USAGE_UNIFORM_BUFFER_BIT |
           VK_BUFFER_USAGE_STORAGE_BUFFER_BIT | VK_BUFFER_USAGE_INDIRECT_BUFFER_BIT | VK_BUFFER_USAGE_TRANSFER_SRC_BIT |
           VK_BUFFER_USAGE_TRANSFER_DST_BIT | VK_BUFFER_USAGE_UNIFORM_TEXEL_BUFFER_BIT | VK_BUFFER_USAGE_STORAGE_TEXEL_BUFFER_BIT |
           (device().features().transform_feedback ? VK_BUFFER_USAGE_TRANSFORM_FEEDBACK_BUFFER_BIT_EXT : 0u);
}

bool covers(const PoolChunk* c, uint64_t off) { return c && c->is_shadow && c->buffer && off >= c->off && off < c->off + c->size; }

}  // namespace

bool pool_copy_on() { return cfg().on; }
uint64_t pool_copy_cell() { return cfg().cell; }
uint32_t pool_copy_idle() { return cfg().idle; }
bool pool_copy_any() { return g_any.load(std::memory_order_relaxed); }

void pcopy_pool_init(PoolGpu* g) {
    const size_t pages = static_cast<size_t>((g->size + kPage - 1) / kPage);
    g->page_gpu.reset(new std::atomic<uint64_t>[pages]);
    for (size_t i = 0; i < pages; ++i) g->page_gpu[i].store(0, std::memory_order_relaxed);
    g_any.store(true, std::memory_order_relaxed);
    start_lander();
}

bool pcopy_alloc(PoolGpu* g, PoolChunk& c, uint64_t off, uint64_t size, const char* what) {
    NXFT("pool.copy.alloc");
    const uint64_t cap = cfg().cap;
    if (cap && static_cast<uint64_t>(std::max<int64_t>(0, g_live.load())) + size > cap) {
        pcopy_evict_for(size);
        static bool logged = false;
        if (static_cast<uint64_t>(std::max<int64_t>(0, g_live.load())) + size > cap && !logged) {
            logged = true;
            NXLOG_I(kTag, "pool_copy: %llu MiB of shadows live, over the cap of %llu MiB, and none idle enough to drop: allocated anyway (logged once)",
                    static_cast<unsigned long long>(g_live.load() >> 20), static_cast<unsigned long long>(cap >> 20));
        }
    }
    gfx::Buffer b;
    if (device().create_buffer(size, shadow_usage(), gfx::MemoryUse::Shadow, &b, "nvn pool shadow") != VK_SUCCESS || !b.mapped) {
        if (b.buffer) device().destroy_buffer(b);
        g_fallback.fetch_add(1, std::memory_order_relaxed);
        static bool warned = false;
        if (!warned) {
            warned = true;
            NXLOG_W(kTag, "pool_copy: a shadow of %llu KiB (pool %p %s at offset 0x%llx) could not be made: imported instead (logged once; counter "
                          "nvn.pool.copy.import_fallback)", static_cast<unsigned long long>(size >> 10), static_cast<void*>(g->base), what,
                    static_cast<unsigned long long>(off));
        }
        return false;
    }
    c.shadow = b;
    c.buffer = b.buffer;
    c.mem = VK_NULL_HANDLE;
    c.view = nullptr;
    c.is_shadow = true;
    c.tried = true;
    c.failed = false;
    c.off = off;
    c.size = size;
    c.owner = g;
    c.page_copied.assign(static_cast<size_t>((size + kPage - 1) / kPage), 0);   // nothing is copied until a bind touches it
    c.land_refs = 0;
    c.cov_serial = c.cov_lo = c.cov_hi = 0;
    {
        std::lock_guard<std::mutex> lk(g_reg_mu);
        g_reg.push_back(&c);
    }
    g_live.fetch_add(static_cast<int64_t>(size), std::memory_order_relaxed);
    g_chunks.fetch_add(1, std::memory_order_relaxed);
    g_allocs.fetch_add(1, std::memory_order_relaxed);
    NXLOG_D(kTag, "pool %p: shadow %s at offset 0x%llx, %llu KiB", static_cast<void*>(g->base), what, static_cast<unsigned long long>(off),
            static_cast<unsigned long long>(size >> 10));
    return true;
}

PoolChunk* pcopy_span(PoolGpu* g, uint64_t off, uint64_t size, bool* made) {
    for (PoolChunk& s : g->spans)
        if (s.is_shadow && s.buffer && off >= s.off && off + size <= s.off + s.size) return &s;
    const uint64_t s0 = off / g->stride * g->stride;
    const uint64_t s1 = std::min<uint64_t>((off + size + g->stride - 1) / g->stride * g->stride, g->size);
    const uint64_t maxb = device().limits_ext().max_buffer_size;
    if (s1 <= s0 || (maxb && s1 - s0 > maxb)) return nullptr;
    PoolChunk* slot = nullptr;
    for (PoolChunk& s : g->spans)                 // a dropped span's empty slot (copy pools never erase spans)
        if (!s.buffer && !s.failed && !s.size) { slot = &s; break; }
    const bool appended = !slot;
    if (appended) { g->spans.emplace_back(); slot = &g->spans.back(); }
    if (!pcopy_alloc(g, *slot, s0, s1 - s0, "span")) {
        if (appended) g->spans.pop_back();
        return nullptr;
    }
    NXLOG_I(kTag, "pool %p: a range of %llu KiB at offset 0x%llx crosses a cell boundary: shadow span of %llu MiB at offset 0x%llx",
            static_cast<void*>(g->base), static_cast<unsigned long long>(size >> 10), static_cast<unsigned long long>(off),
            static_cast<unsigned long long>((s1 - s0) >> 20), static_cast<unsigned long long>(s0));
    g_spans.fetch_add(1, std::memory_order_relaxed);
    if (made) *made = true;
    return slot;
}

void pcopy_free(PoolChunk& c) {
    reg_remove(&c);
    g_free_gen.fetch_add(1, std::memory_order_acq_rel);
    std::lock_guard<std::mutex> landing(g_landing_mu);     // no landing copies into this chunk's shadow while it is destroyed
    {   // records of a chunk that goes (its pool is destroyed) do not land; an idle drop or an eviction happens only with none left
        std::lock_guard<std::mutex> lk(g_land_mu);
        g_land.erase(std::remove_if(g_land.begin(), g_land.end(), [&c](const LandRec& r) { return r.chunk == &c; }), g_land.end());
        land_min_update();
    }
    if (t_last.chunk == &c) t_last = LastResolve{};
    g_live.fetch_sub(static_cast<int64_t>(c.size), std::memory_order_relaxed);
    g_chunks.fetch_sub(1, std::memory_order_relaxed);
    device().destroy_buffer(c.shadow);
}

bool pcopy_busy(const PoolChunk& c) {
    std::lock_guard<std::mutex> lk(g_land_mu);
    return c.land_refs != 0;
}

void pcopy_touch(PoolGpu* g, PoolChunk* c, uint64_t off, uint64_t size) {
    const uint64_t cend = c->off + c->size;
    if (off < c->off || off >= cend) return;
    const uint64_t end = size ? std::min<uint64_t>(off + size, cend) : cend;   // unknown size: to the end of the chunk
    const uint64_t p0 = (off - c->off) / kPage, p1 = (end - c->off + kPage - 1) / kPage;   // chunk pages [p0, p1)
    const bool rt = on_render_thread();
    if (rt) t_last = LastResolve{g, c, g_free_gen.load(std::memory_order_acquire)};
    const uint64_t cur = rt ? recording_serial() : 0;     // 0 outside a recording: always copied
    if (cur && c->cov_serial == cur && p0 >= c->cov_lo && p1 <= c->cov_hi) return;   // all copied in this submission already
    const uint64_t done = completed_serial();
    const uint64_t pbase = c->off / kPage;
    // 0: copied in this submission; 1: copy; 2: a GPU write is in flight there (the shadow is newer), unless nothing was ever copied into
    // the page of this chunk (a new span over a page another chunk writes): then guest memory is the better source.
    const auto need = [&](uint64_t p) -> int {
        if (cur && c->page_copied[static_cast<size_t>(p)] == cur) return 0;
        if (g->page_gpu[static_cast<size_t>(pbase + p)].load(std::memory_order_acquire) > done && (!rt || c->page_copied[static_cast<size_t>(p)] != 0)) return 2;
        return 1;
    };
    uint8_t* dst = static_cast<uint8_t*>(c->shadow.mapped);
    const uint8_t* src = g->base + c->off;
    const uint64_t t0 = ft::on() ? ft::now_ns() : 0;
    uint64_t bytes = 0;
    bool all = true;
    for (uint64_t p = p0; p < p1;) {
        const int n = need(p);
        if (n != 1) {
            if (n == 2) all = false;
            ++p;
            continue;
        }
        uint64_t q = p + 1;
        while (q < p1 && need(q) == 1) ++q;
        const uint64_t bad = guarded_copy(dst + p * kPage, src + p * kPage, (q - p) * kPage, true);
        if (bad) g_unreadable.fetch_add(bad, std::memory_order_relaxed);
        if (rt)
            for (uint64_t i = p; i < q; ++i) c->page_copied[static_cast<size_t>(i)] = cur ? cur : kOutside;
        bytes += (q - p) * kPage;
        p = q;
    }
    if (cur && all) {      // [p0, p1) is current for this submission: widen the fast path's interval
        if (c->cov_serial == cur && p0 <= c->cov_hi && p1 >= c->cov_lo) {
            c->cov_lo = std::min(c->cov_lo, p0);
            c->cov_hi = std::max(c->cov_hi, p1);
        } else {
            c->cov_serial = cur;
            c->cov_lo = p0;
            c->cov_hi = p1;
        }
    }
    if (!bytes) return;
    g_copied.fetch_add(bytes, std::memory_order_relaxed);
    if (rt) {
        if (cur != g_sub_serial) {
            g_last_sub_kb.store(static_cast<int64_t>(g_sub_bytes >> 10), std::memory_order_relaxed);
            g_sub_serial = cur;
            g_sub_bytes = 0;
        }
        g_sub_bytes += bytes;
    }
    if (t0) {
        static const uint16_t z = ft::zone_id("pool.copy.touch");
        ft::event(z, t0, ft::now_ns(), static_cast<uint32_t>(std::min<uint64_t>(bytes >> 10, 0xffffffffu)));
    }
}

void pool_copy_touch(uint64_t gpu_address, uint64_t size) {
    if (!pool_copy_on() || !size || !last_valid()) return;
    const uint64_t off = gpu_address - reinterpret_cast<uint64_t>(t_last.pool->base);
    if (!covers(t_last.chunk, off)) return;
    pcopy_touch(t_last.pool, t_last.chunk, off, size);
}

void pcopy_note_write(uint64_t gpu_address, uint64_t size, VkBuffer hint) {
    if (!size) return;
    const PoolHit hit = pool_lookup(gpu_address);
    if (!hit.pool || !hit.pool->gpu) return;
    PoolGpu* pg = hit.pool->gpu;
    if (!pg->copy || pg->failed || !pg->page_gpu) return;
    const uint64_t off = gpu_address - reinterpret_cast<uint64_t>(pg->base);
    if (off >= pg->size) return;
    PoolChunk* c = nullptr;
    bool span = false;
    if (hint) {
        for (PoolChunk& x : pg->ext) if (x.buffer == hint && covers(&x, off)) { c = &x; break; }
        if (!c) for (PoolChunk& x : pg->spans) if (x.buffer == hint && covers(&x, off)) { c = &x; span = true; break; }
    }
    if (!c && last_valid() && t_last.pool == pg && covers(t_last.chunk, off) && (!hint || t_last.chunk->buffer == hint)) {
        c = t_last.chunk;
        for (const PoolChunk& x : pg->spans) if (&x == c) { span = true; break; }
    }
    if (!c) {     // not the buffer the last resolve gave: the cell's chunk, else a span holding the address
        const int32_t e = pg->cell_ext[static_cast<size_t>(off / pg->stride)];
        if (e >= 0 && covers(&pg->ext[static_cast<size_t>(e)], off)) c = &pg->ext[static_cast<size_t>(e)];
        if (!c) for (PoolChunk& x : pg->spans) if (covers(&x, off)) { c = &x; span = true; break; }
        if (c) g_unmatched.fetch_add(1, std::memory_order_relaxed);
    }
    if (!c) return;     // an imported fallback chunk (the GPU writes guest memory itself) or nothing resolved there
    const uint64_t end = std::min<uint64_t>({off + size, pg->size, c->off + c->size});
    const uint64_t serial = recording_serial() ? recording_serial() : current_serial();
    if (span) g_span_writes.fetch_add(1, std::memory_order_relaxed);
    for (uint64_t p = off / kPage; p <= (end - 1) / kPage; ++p) {
        std::atomic<uint64_t>& w = pg->page_gpu[static_cast<size_t>(p)];
        if (w.load(std::memory_order_relaxed) < serial) w.store(serial, std::memory_order_release);
    }
    bool first = false;
    {
        std::lock_guard<std::mutex> lk(g_land_mu);
        // the same range bound again in this submission (a storage buffer of every draw): one record
        const size_t n = g_land.size();
        for (size_t i = n; i-- > 0 && i + 8 >= n;) {
            const LandRec& r = g_land[i];
            if (r.serial != serial) break;
            if (r.chunk == c && r.pool_off <= off && end <= r.pool_off + r.size) return;
        }
        first = g_land.empty();
        g_land.push_back(LandRec{pg, c, off, end - off, serial});
        ++c->land_refs;
        if (first) land_min_update();
    }
    g_records.fetch_add(1, std::memory_order_relaxed);
    if (first) {
        std::lock_guard<std::mutex> lk(g_th_mu);     // under the mutex the lander waits on: no wake-up lost between its test and its wait
        g_th_cv.notify_one();
    }
}

uint64_t pcopy_land(uint64_t reached) {
    if (t_landing || g_land_min.load(std::memory_order_acquire) > reached) return reached;
    t_landing = true;
    const uint64_t t0 = ft::on() ? ft::now_ns() : 0;
    uint64_t bytes = 0, bad = 0;
    {
        std::lock_guard<std::mutex> landing(g_landing_mu);
        // The records leave the queue now, but g_land_min and land_refs stay as they were until the copy is done: a second caller that tests
        // g_land_min still waits on g_landing_mu, so no serial is published before its bytes landed, and pcopy_busy holds the chunks.
        std::vector<LandRec> batch;
        {
            std::lock_guard<std::mutex> lk(g_land_mu);
            while (!g_land.empty() && g_land.front().serial <= reached) {
                batch.push_back(g_land.front());
                g_land.pop_front();
            }
        }
        // pass 1: every copy. Two records of a serial can share a page: its page_gpu must stay set until both have landed, or a touch between
        // them sees 0 and copies guest -> shadow over the page while the second record still lands stale bytes.
        for (const LandRec& r : batch) {
            const PoolChunk* c = r.chunk;
            uint8_t* dst = r.pool->base + r.pool_off;
            const uint8_t* src = static_cast<const uint8_t*>(c->shadow.mapped) + (r.pool_off - c->off);
            mem::ww::before_os_write(dst, r.size);     // pages a write watch made read-only become writable (and count as written)
            bad += guarded_copy(dst, src, r.size, false);
            bytes += r.size;
        }
        // pass 2: page_gpu back to 0 where no later write was recorded: after every copy, so a touch that sees 0 copies the landed bytes
        for (const LandRec& r : batch)
            for (uint64_t p = r.pool_off / kPage; p <= (r.pool_off + r.size - 1) / kPage; ++p) {
                uint64_t e = r.serial;
                r.pool->page_gpu[static_cast<size_t>(p)].compare_exchange_strong(e, 0, std::memory_order_release, std::memory_order_relaxed);
            }
        {
            std::lock_guard<std::mutex> lk(g_land_mu);
            for (const LandRec& r : batch)
                if (r.chunk->land_refs) --r.chunk->land_refs;
            land_min_update();
        }
    }
    t_landing = false;
    if (bad) g_unreadable.fetch_add(bad, std::memory_order_relaxed);
    if (bytes) {
        g_landed.fetch_add(bytes, std::memory_order_relaxed);
        if (t0) {
            static const uint16_t z = ft::zone_id("pool.copy.land");
            ft::event(z, t0, ft::now_ns(), static_cast<uint32_t>(std::min<uint64_t>(bytes >> 10, 0xffffffffu)));
        }
    }
    return reached;
}

void pcopy_shutdown() {
    {
        std::lock_guard<std::mutex> lk(g_th_mu);
        g_th_stop = true;
    }
    g_th_cv.notify_all();
    if (g_th.joinable()) g_th.join();
    {
        std::lock_guard<std::mutex> lk(g_th_mu);
        g_th_stop = false;            // a later back end (tests bring it up and down) starts its own
    }
    drop_all_records();               // the device is idle and the pools go next
    t_last = LastResolve{};
}

void pcopy_evict_for(uint64_t bytes) {
    NXFT("pool.copy.evict");
    const uint64_t cap = cfg().cap;
    const uint64_t done = poll_completed(), cur = current_serial();
    const uint32_t now = static_cast<uint32_t>(counters().presents.load(std::memory_order_relaxed));
    std::vector<PoolChunk*> cand;
    {
        std::lock_guard<std::mutex> lk(g_reg_mu);
        for (PoolChunk* c : g_reg)
            if (c->buffer && c->used_serial <= done && c->used_serial < cur) cand.push_back(c);
    }
    std::sort(cand.begin(), cand.end(), [now](const PoolChunk* a, const PoolChunk* b) { return now - a->used_at > now - b->used_at; });
    uint64_t freed = 0;
    for (PoolChunk* c : cand) {
        if (static_cast<uint64_t>(std::max<int64_t>(0, g_live.load())) + bytes <= cap) break;
        if (pcopy_busy(*c)) continue;
        const uint64_t sz = c->size;
        if (!pool_drop_chunk_if_live(c->owner, c)) continue;     // under g_live_mu, like lazy_drop_idle
        freed += sz;
        g_evicted.fetch_add(1, std::memory_order_relaxed);
    }
    if (freed)
        NXLOG_I(kTag, "pool_copy: over the cap of %llu MiB: dropped %llu MiB of the least recently used shadows", static_cast<unsigned long long>(cap >> 20),
                static_cast<unsigned long long>(freed >> 20));
}

void pcopy_log_summary(uint32_t presents) {
    (void)presents;
    static auto last = std::chrono::steady_clock::now();
    static uint64_t p_copied = 0, p_landed = 0;
    const auto now = std::chrono::steady_clock::now();
    const double secs = std::max(1e-3, std::chrono::duration<double>(now - last).count());
    last = now;
    const uint64_t copied = g_copied.load(), landed = g_landed.load();
    size_t pending = 0;
    {
        std::lock_guard<std::mutex> lk(g_land_mu);
        pending = g_land.size();
    }
    NXLOG_I(kTag, "pool_copy: %lld MiB of shadows live in %llu chunks (%llu spans made, %llu dropped by the cap); copied in %.1f MiB/s, landed %.1f MiB/s; "
                  "%llu writes recorded, %zu waiting; %llu import fallbacks, %llu unreadable pages, %llu writes matched by cell, %llu span writes",
            static_cast<long long>(g_live.load() >> 20), static_cast<unsigned long long>(g_chunks.load()), static_cast<unsigned long long>(g_spans.load()),
            static_cast<unsigned long long>(g_evicted.load()), (copied - p_copied) / 1048576.0 / secs, (landed - p_landed) / 1048576.0 / secs,
            static_cast<unsigned long long>(g_records.load()), pending, static_cast<unsigned long long>(g_fallback.load()),
            static_cast<unsigned long long>(g_unreadable.load()), static_cast<unsigned long long>(g_unmatched.load()),
            static_cast<unsigned long long>(g_span_writes.load()));
    p_copied = copied;
    p_landed = landed;
}

PoolCopyStats pool_copy_stats() {
    PoolCopyStats s;
    s.on = pool_copy_on();
    s.live_bytes = g_live.load();
    s.chunks = g_chunks.load();
    s.allocs = g_allocs.load();
    s.copied_bytes = g_copied.load();
    s.landed_bytes = g_landed.load();
    s.records = g_records.load();
    {
        std::lock_guard<std::mutex> lk(g_land_mu);
        s.pending = g_land.size();
    }
    s.write_unmatched = g_unmatched.load();
    s.import_fallback = g_fallback.load();
    s.unreadable = g_unreadable.load();
    s.span_writes = g_span_writes.load();
    s.evicted = g_evicted.load();
    return s;
}

}  // namespace nxrt::nvn::be::gpu
