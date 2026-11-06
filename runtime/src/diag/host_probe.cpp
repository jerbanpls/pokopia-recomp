// SPDX-License-Identifier: GPL-3.0-or-later
// Host probe: see host_probe.h.
#include "host_probe.h"

#include <windows.h>

#include <chrono>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <string>

#include "nxrt/config.h"
#include "nxrt/log.h"
#include "nxrt/memory.h"

namespace nxrt::diag {
namespace {

constexpr const char* kTag = "host";

bool under_wine() {
    HMODULE nt = GetModuleHandleW(L"ntdll.dll");
    return nt && GetProcAddress(nt, "wine_get_version");
}

void run_probe() {
    std::ifstream sm("Z:\\proc\\self\\smaps");
    std::string thp;
    {
        std::ifstream f("Z:\\sys\\kernel\\mm\\transparent_hugepage\\enabled");
        if (f) std::getline(f, thp);
        while (!thp.empty() && (thp.back() == '\r' || thp.back() == '\n' || thp.back() == ' ')) thp.pop_back();
    }
    if (!sm || thp.empty()) {
        NXLOG_I(kTag, "host probe: not available");
        return;
    }
    // smaps: a header line "start-end perms ..." per mapping, then "Key: value kB" lines. The heap is the mapping that holds kHeapBase.
    const unsigned long long want = static_cast<unsigned long long>(mem::kHeapBase);
    std::string line;
    bool in_heap = false, found = false;
    long long rss = -1, anon_huge = -1, shmem_pmd = -1, file_pmd = -1;
    unsigned long long lo = 0, hi = 0;
    while (std::getline(sm, line)) {
        unsigned long long a = 0, b = 0;
        if (std::sscanf(line.c_str(), "%llx-%llx ", &a, &b) == 2) {
            if (found) break;       // the heap mapping is complete
            in_heap = want >= a && want < b;
            if (in_heap) { found = true; lo = a; hi = b; }
            continue;
        }
        if (!in_heap) continue;
        long long v = 0;
        if (std::sscanf(line.c_str(), "Rss: %lld kB", &v) == 1) rss = v;
        else if (std::sscanf(line.c_str(), "AnonHugePages: %lld kB", &v) == 1) anon_huge = v;
        else if (std::sscanf(line.c_str(), "ShmemPmdMapped: %lld kB", &v) == 1) shmem_pmd = v;
        else if (std::sscanf(line.c_str(), "FilePmdMapped: %lld kB", &v) == 1) file_pmd = v;
    }
    if (!found) {
        NXLOG_I(kTag, "host probe: THP mode '%s'; no mapping holds the heap base %llx", thp.c_str(), want);
        return;
    }
    NXLOG_I(kTag, "host probe: heap mapping %llx-%llx: Rss %lld kB, AnonHugePages %lld kB, ShmemPmdMapped %lld kB, FilePmdMapped %lld kB; THP mode '%s'",
            lo, hi, rss, anon_huge, shmem_pmd, file_pmd, thp.c_str());
}

}  // namespace

void host_probe_tick(bool presented) {
    using Clock = std::chrono::steady_clock;
    static bool done = false;                 // the hitch monitor thread is the only caller
    static Clock::time_point first{};
    if (done || !presented) return;
    if (!config_get_bool("diag", "stutter_logs", true)) { done = true; return; }
    if (first == Clock::time_point{}) first = Clock::now();
    if (Clock::now() - first < std::chrono::seconds(60)) return;
    done = true;
    if (!under_wine()) return;
    run_probe();
}

}  // namespace nxrt::diag
