// SPDX-License-Identifier: GPL-3.0-or-later
// [nvn] one_submit: batch flushes handed to the submit worker (nvn_submit_worker.h) wait here until one of two owners takes them:
// - their own worker job, which submits them on their own (a kernel submission of their own), or
// - the next present, while the job has not started yet: they ride in the present's vkQueueSubmit2 (one kernel submission for both).
// Under Proton each kernel submission costs ~15 ms on a Steam Deck (the userptr pool imports are validated again), and the game's fence polls
// flush part of every frame early (~1 extra submission per frame), so taking those flushes into the present halves the worker's time.
//
// Contract (what keeps the GPU order and the waits right):
// - Entries are parked in increasing serial order (the render thread's flush order) and every entry has exactly one owner.
// - The present takes the entries that are not released, i.e. a suffix of the parked ones (release is a serial threshold). The released
//   prefix stays for its jobs, which were pushed earlier and so run on the worker before the present's job: the queue still sees the
//   serials in increasing order.
// - release(serial): a CPU wait for `serial` needs it on the queue now: every entry that holds a serial <= `serial` goes with its own job,
//   at once (a holding job wakes), also one parked after the call. release_all(): a drain, the same for everything parked now. The present never takes a
//   released entry.
// - Hold: a job may wait up to `hold` (counted from the park time) for the present to take its entry, but only when a present is already
//   queued on the render thread (present_coming). It ends early on release or when the present takes the entry.
// No Vulkan in here.
#pragma once

#include <algorithm>
#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <deque>
#include <mutex>
#include <optional>
#include <utility>
#include <vector>

namespace nxrt::nvn::be::gpu {

template <class Payload>
class SubmitPark {
public:
    using Clock = std::chrono::steady_clock;
    struct Entry {
        uint64_t first = 0;     // the flush carries serials first..serial (it signals `serial`, the earlier ones complete with it)
        uint64_t serial = 0;
        Payload payload{};
        Clock::time_point parked{};
    };
    // How a job's take ended (the zone argument of submit.park.hold).
    enum class Took { Now = 0, Present = 1, Released = 2, Expired = 3 };

    // Render thread: a flushed batch with serials first..serial (ranges increase from call to call and do not overlap).
    void park(uint64_t first, uint64_t serial, Payload p) {
        std::lock_guard<std::mutex> lk(mu_);
        q_.push_back(Entry{std::min(first, serial), serial, std::move(p), Clock::now()});
    }

    // The worker job of `serial`. Returns the payload when the job is to submit it (Now / Released / Expired), nullopt when the present took
    // it (Present). hold > 0 and present_coming: waits as described above.
    std::optional<Payload> take_for_job(uint64_t serial, std::chrono::nanoseconds hold, bool present_coming, Took* how = nullptr) {
        std::unique_lock<std::mutex> lk(mu_);
        auto it = find(serial);
        Took t = Took::Now;
        if (it == q_.end()) {
            if (how) *how = Took::Present;
            return std::nullopt;
        }
        if (hold.count() > 0 && present_coming && !released(*it)) {
            const Clock::time_point until = it->parked + std::chrono::duration_cast<Clock::duration>(hold);
            cv_.wait_until(lk, until, [&] { auto i = find(serial); return i == q_.end() || released(*i); });
            it = find(serial);
            if (it == q_.end()) {
                if (how) *how = Took::Present;
                return std::nullopt;
            }
            t = released(*it) ? Took::Released : Took::Expired;
        }
        std::optional<Payload> out(std::move(it->payload));
        q_.erase(it);
        if (how) *how = t;
        return out;
    }

    // The present (render thread): every entry not released, in serial order. Holding jobs of those entries wake and find them gone.
    std::vector<Entry> take_for_present() {
        std::vector<Entry> out;
        {
            std::lock_guard<std::mutex> lk(mu_);
            auto first = std::find_if(q_.begin(), q_.end(), [&](const Entry& e) { return !released(e); });
            for (auto it = first; it != q_.end(); ++it) out.push_back(std::move(*it));
            q_.erase(first, q_.end());
        }
        if (!out.empty()) cv_.notify_all();
        return out;
    }

    // A CPU wait for `serial`: entries holding a serial <= `serial` go with their own jobs at once, also ones parked after this call (the
    // wait may come before the render thread has flushed its serial).
    void release(uint64_t serial) {
        {
            std::lock_guard<std::mutex> lk(mu_);
            if (serial <= released_) return;
            released_ = serial;
        }
        cv_.notify_all();
    }

    // A drain: everything parked now goes with its own job (a later park can be held again).
    void release_all() {
        uint64_t last = 0;
        {
            std::lock_guard<std::mutex> lk(mu_);
            if (q_.empty()) return;
            last = q_.back().serial;
        }
        release(last);
    }

    size_t size() {
        std::lock_guard<std::mutex> lk(mu_);
        return q_.size();
    }

private:
    bool released(const Entry& e) const { return e.first <= released_; }    // a wait wants one of its serials
    typename std::deque<Entry>::iterator find(uint64_t serial) {
        return std::find_if(q_.begin(), q_.end(), [&](const Entry& e) { return e.serial == serial; });
    }

    std::mutex mu_;
    std::condition_variable cv_;    // holding jobs: their entry was taken or released
    std::deque<Entry> q_;           // under mu_, increasing serials
    uint64_t released_ = 0;         // under mu_
};

}  // namespace nxrt::nvn::be::gpu
