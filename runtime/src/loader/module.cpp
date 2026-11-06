// SPDX-License-Identifier: GPL-3.0-or-later
// Module loader and linker (see include/nxrt/module.h).
// Pattern from smo-recomp runtime/core/hostrt.c runtime_init (relocation handling, post-relocation page protection) and
// SuperMarioWonderRecomp runtime/src/loader.cpp (map every module first, then link; import thunks and logging stubs).
#include "nxrt/module.h"

#include <algorithm>
#include <cstdarg>
#include <cstdio>
#include <cstring>
#include <fstream>
#include <functional>
#include <map>
#include <memory>
#include <mutex>
#include <set>
#include <unordered_map>

#include "elf.h"
#include "nxrt/dispatch.h"
#include "nxrt/gauge.h"
#include "nxrt/log.h"
#include "nxrt/memory.h"
#include "nxrt/nxmodule.h"

namespace nxrt {

namespace {
constexpr const char* kTag = "loader";
std::recursive_mutex g_lock;
std::vector<std::unique_ptr<Module>> g_modules;
std::unordered_map<std::string, uint64_t> g_data;
std::set<std::string> g_overrides;
uint64_t g_next_base = 0;
TlsInfo g_tls;

// ---- gauges (include/nxrt/gauge.h); each callback takes g_lock, the lock of the data it counts ----
// Removes its gauge when this file's statics are torn down (declared after them, so it goes first).
struct GaugeHook {
    const char* name;
    GaugeHook(const char* n, std::function<int64_t()> f) : name(n) { nxrt::gauge_register(n, std::move(f)); }
    ~GaugeHook() { nxrt::gauge_unregister(name); }
};
GaugeHook gauge_modules("loader.modules", [] {
    std::lock_guard<std::recursive_mutex> lk(g_lock);
    return static_cast<int64_t>(g_modules.size());
});
GaugeHook gauge_data_symbols("loader.data_symbols", [] {   // data symbols the host has published (g_data only grows until reset)
    std::lock_guard<std::recursive_mutex> lk(g_lock);
    return static_cast<int64_t>(g_data.size());
});
GaugeHook gauge_overrides("loader.overrides", [] {
    std::lock_guard<std::recursive_mutex> lk(g_lock);
    return static_cast<int64_t>(g_overrides.size());
});

uint64_t align_up(uint64_t v, uint64_t a) { return (v + a - 1) & ~(a - 1); }
uint64_t align_down(uint64_t v, uint64_t a) { return v & ~(a - 1); }

bool fail(std::string* err, const char* fmt, ...) {
    char buf[512];
    va_list ap;
    va_start(ap, fmt);
    vsnprintf(buf, sizeof buf, fmt, ap);
    va_end(ap);
    if (err) *err = buf;
    NXLOG_E(kTag, "%s", buf);
    return false;
}
}  // namespace

// Gives the helper functions below access to Module's private state.
struct LoaderAccess {
    static Module* create_and_map(const uint8_t* d, size_t n, const LoadOptions& opt, std::string* err);
    static bool parse_dynamic(Module& m, const uint8_t* d, size_t n, std::string* err);
    static void load_static_symtab(Module& m, const uint8_t* d, size_t n, const elf::Ehdr& eh);
    static bool link(Module& m, bool guest_relinks = false);
    static void protect(Module& m);
    static bool has_relr(const Module& m) { return m.relr_ && m.relr_sz_; }
    static uint64_t resolve(Module& m, const std::vector<elf::Sym>& raw, uint32_t symidx, bool* is_weak_null);
};

// ---- Module ----------------------------------------------------------------------------------------------------------

uint64_t Module::find_symbol(const char* sym) const {
    std::call_once(index_once_, [this] {          // the game's modules define 100k symbols and import thousands: no linear scans
        index_.reserve(symbols_.size());
        for (uint32_t i = 0; i < symbols_.size(); i++)
            if (symbols_[i].defined() && symbols_[i].bind != 0 && !symbols_[i].name.empty()) index_.emplace(symbols_[i].name, i);
    });
    auto it = index_.find(sym);
    if (it == index_.end()) return 0;
    const ModuleSymbol& s = symbols_[it->second];
    return s.shndx == elf::SHN_ABS ? s.value : bias_ + s.value;
}

std::string Module::describe(uint64_t addr) const {
    const ModuleSymbol* best = nullptr;
    uint64_t best_addr = 0;
    for (auto& s : symbols_) {
        if (!s.defined() || s.name.empty() || (s.type != 2 && s.type != 1 && s.type != 0) || s.shndx == elf::SHN_ABS) continue;
        uint64_t a = bias_ + s.value;
        if (a <= addr && (!best || a > best_addr)) {
            best = &s;
            best_addr = a;
        }
    }
    if (!best) return {};
    char buf[256];
    snprintf(buf, sizeof buf, "%s+0x%llx", best->name.c_str(), static_cast<unsigned long long>(addr - best_addr));
    return buf;
}

void Module::set_build_id(const uint8_t (&id)[32]) {
    static const char kHex[] = "0123456789abcdef";
    build_id_.resize(64);
    for (int i = 0; i < 32; i++) {
        build_id_[2 * i] = kHex[id[i] >> 4];
        build_id_[2 * i + 1] = kHex[id[i] & 15];
    }
}

// ---- phase 1: map and parse --------------------------------------------------------------------------------------------

Module* LoaderAccess::create_and_map(const uint8_t* d, size_t n, const LoadOptions& opt, std::string* err) {
    using namespace elf;
    if (n < sizeof(Ehdr)) { fail(err, "not an ELF image (%zu bytes)", n); return nullptr; }
    Ehdr eh;
    memcpy(&eh, d, sizeof eh);
    if (memcmp(eh.ident, "\x7f" "ELF", 4) != 0 || eh.ident[4] != 2 || eh.ident[5] != 1) { fail(err, "not a little-endian ELF64 image"); return nullptr; }
    if (eh.machine != EM_AARCH64) { fail(err, "ELF machine %u is not AArch64", eh.machine); return nullptr; }
    if (eh.type != ET_DYN && eh.type != ET_EXEC) { fail(err, "ELF type %u is neither ET_DYN nor ET_EXEC", eh.type); return nullptr; }
    if (eh.phentsize != sizeof(Phdr) || eh.phnum == 0 || eh.phnum > 256) { fail(err, "bad program header table (entsize %u, count %u)", eh.phentsize, eh.phnum); return nullptr; }
    if (eh.phoff > n || uint64_t(eh.phnum) * sizeof(Phdr) > n - eh.phoff) { fail(err, "program header table is outside the file"); return nullptr; }

    std::vector<Phdr> ph(eh.phnum);
    memcpy(ph.data(), d + eh.phoff, ph.size() * sizeof(Phdr));

    auto mod = std::make_unique<Module>();
    Module& m = *mod;
    m.name_ = opt.name;
    m.entry_ = eh.entry;
    uint64_t lo = ~0ull, hi = 0;
    for (auto& p : ph) {
        if (p.type == PT_LOAD) {
            if (p.memsz < p.filesz) { fail(err, "PT_LOAD with memsz < filesz"); return nullptr; }
            if (p.offset > n || p.filesz > n - p.offset) { fail(err, "PT_LOAD data (offset 0x%llx, size 0x%llx) is outside the file", (unsigned long long)p.offset, (unsigned long long)p.filesz); return nullptr; }
            if (p.vaddr + p.memsz < p.vaddr || p.vaddr > (1ull << 40)) { fail(err, "PT_LOAD address range is absurd"); return nullptr; }
            m.segs_.push_back({p.vaddr, p.memsz, p.filesz, p.offset, p.flags});
            lo = std::min(lo, align_down(p.vaddr, mem::kPage));
            hi = std::max(hi, align_up(p.vaddr + p.memsz, mem::kPage));
        } else if (p.type == PT_TLS) {
            m.tls_.file_size = p.filesz;
            m.tls_.mem_size = p.memsz;
            m.tls_.align = p.align ? p.align : 1;
            m.tls_.init_addr = p.vaddr;      // made absolute below
            m.tls_.tp_offset = align_up(16, m.tls_.align);
        } else if (p.type == PT_GNU_RELRO) {
            m.relro_start_ = p.vaddr;
            m.relro_size_ = p.memsz;
        }
    }
    if (m.segs_.empty()) { fail(err, "no PT_LOAD segments"); return nullptr; }
    if (hi - lo > opt.max_image_size) { fail(err, "image span 0x%llx exceeds the limit 0x%llx", (unsigned long long)(hi - lo), (unsigned long long)opt.max_image_size); return nullptr; }

    uint64_t base;
    if (eh.type == ET_EXEC) {
        if (opt.base && opt.base != lo) { fail(err, "ET_EXEC image is linked at 0x%llx, cannot load at 0x%llx", (unsigned long long)lo, (unsigned long long)opt.base); return nullptr; }
        base = lo;
        m.bias_ = 0;
    } else {
        base = opt.base;
        if (!base) base = g_modules.empty() ? mem::kMainBase : std::max(g_next_base, mem::kSdkBase);
        base = align_up(base, mem::kAllocGranularity);
        m.bias_ = base - lo;       // lo is normally 0
    }
    m.base_ = base;
    m.size_ = hi - lo;
    if (m.name_.empty()) m.name_ = "module" + std::to_string(g_modules.size());
    m.entry_ = eh.entry ? m.bias_ + eh.entry : 0;
    if (m.tls_.present()) m.tls_.init_addr += m.bias_;

    for (auto& other : g_modules)
        if (base < other->base() + other->size() && other->base() < base + m.size_) { fail(err, "module %s at 0x%llx overlaps %s", m.name_.c_str(), (unsigned long long)base, other->name().c_str()); return nullptr; }

    if (!mem::reserve_fixed(base, align_up(m.size_, mem::kAllocGranularity), m.name_.c_str())) { fail(err, "cannot reserve 0x%llx bytes at 0x%llx for %s (range in use)", (unsigned long long)m.size_, (unsigned long long)base, m.name_.c_str()); return nullptr; }
    if (!mem::commit(base, m.size_, mem::Prot::RW)) { mem::release_fixed(base); fail(err, "cannot commit the image of %s", m.name_.c_str()); return nullptr; }
    for (auto& s : m.segs_)
        if (s.filesz) memcpy(reinterpret_cast<void*>(m.bias_ + s.vaddr), d + s.offset, s.filesz);   // bss and the segment tail are already zero

    if (!parse_dynamic(m, d, n, err)) { mem::release_fixed(base); return nullptr; }
    g_next_base = base + align_up(m.size_, mem::kAllocGranularity);
    NXLOG_I(kTag, "mapped %s at 0x%llx (+0x%llx bytes, %zu segments, %zu symbols, entry 0x%llx)", m.name_.c_str(), (unsigned long long)base,
            (unsigned long long)m.size_, m.segs_.size(), m.symbols_.size(), (unsigned long long)m.entry_);
    g_modules.push_back(std::move(mod));
    return g_modules.back().get();
}

// A static ET_EXEC image has no dynamic section but usually keeps .symtab: take the defined symbols from it (names for backtraces,
// module_find_symbol for tests and for the C++ typeinfo vtables, eh.h). Reads the file bytes, not the mapped image.
void LoaderAccess::load_static_symtab(Module& m, const uint8_t* d, size_t n, const elf::Ehdr& eh) {
    using namespace elf;
    if (!eh.shoff || !eh.shnum || eh.shentsize != sizeof(Shdr) || eh.shoff > n || uint64_t(eh.shnum) * sizeof(Shdr) > n - eh.shoff) return;
    std::vector<Shdr> sh(eh.shnum);
    memcpy(sh.data(), d + eh.shoff, sh.size() * sizeof(Shdr));
    for (const Shdr& s : sh) {
        if (s.type != SHT_SYMTAB || s.entsize != sizeof(Sym) || s.offset > n || s.size > n - s.offset || s.link >= sh.size()) continue;
        const Shdr& str = sh[s.link];
        if (str.offset > n || str.size > n - str.offset) continue;
        size_t count = static_cast<size_t>(s.size / sizeof(Sym));
        m.symbols_.reserve(count);
        for (size_t i = 0; i < count; i++) {
            Sym sym;
            memcpy(&sym, d + s.offset + i * sizeof(Sym), sizeof sym);
            if (sym.shndx == 0 || (sym.info & 0xf) > 6 || sym.name >= str.size) continue;     // skip undefined and section/file symbols
            if ((sym.info & 0xf) == 3 || (sym.info & 0xf) == 4) continue;
            const char* p = reinterpret_cast<const char*>(d + str.offset) + sym.name;
            ModuleSymbol ms;
            ms.name.assign(p, strnlen(p, static_cast<size_t>(str.size - sym.name)));
            ms.value = sym.value;
            ms.size = sym.size;
            ms.type = sym.info & 0xf;
            ms.bind = sym.info >> 4;
            ms.shndx = sym.shndx;
            m.symbols_.push_back(std::move(ms));
        }
        return;
    }
}

bool LoaderAccess::parse_dynamic(Module& m, const uint8_t* d, size_t n, std::string* err) {
    using namespace elf;
    const uint64_t lo = m.base_ - m.bias_;    // lowest mapped ELF vaddr (page aligned)
    const uint64_t hi = lo + m.size_;
    auto in_image = [&](uint64_t vaddr, uint64_t len) { return vaddr >= lo && vaddr <= hi && len <= hi - vaddr; };
    auto at = [&](uint64_t vaddr) { return reinterpret_cast<const uint8_t*>(m.bias_ + vaddr); };

    Ehdr eh;
    memcpy(&eh, d, sizeof eh);
    std::vector<Phdr> ph(eh.phnum);
    memcpy(ph.data(), d + eh.phoff, ph.size() * sizeof(Phdr));
    uint64_t dyn_vaddr = 0, dyn_size = 0;
    for (auto& p : ph)
        if (p.type == PT_DYNAMIC) { dyn_vaddr = p.vaddr; dyn_size = p.memsz; }
    if (!dyn_vaddr) {                  // a static image: nothing to relocate; its .symtab (when the section headers are there) names the code
        load_static_symtab(m, d, n, eh);
        return true;
    }
    if (!in_image(dyn_vaddr, dyn_size)) return fail(err, "PT_DYNAMIC is outside the image");

    uint64_t symtab = 0, strtab = 0, strsz = 0, hash = 0, gnu_hash = 0, syment = sizeof(Sym);
    for (uint64_t off = 0; off + sizeof(Dyn) <= dyn_size; off += sizeof(Dyn)) {
        Dyn dn;
        memcpy(&dn, at(dyn_vaddr + off), sizeof dn);
        if (dn.tag == DT_NULL) break;
        switch (dn.tag) {
            case DT_SYMTAB: symtab = dn.val; break;
            case DT_STRTAB: strtab = dn.val; break;
            case DT_STRSZ: strsz = dn.val; break;
            case DT_SYMENT: syment = dn.val; break;
            case DT_HASH: hash = dn.val; break;
            case DT_GNU_HASH: gnu_hash = dn.val; break;
            case DT_RELA: m.rela_ = dn.val; break;
            case DT_RELASZ: m.rela_sz_ = dn.val; break;
            case DT_JMPREL: m.jmprel_ = dn.val; break;
            case DT_PLTRELSZ: m.jmprel_sz_ = dn.val; break;
            case DT_RELR: m.relr_ = dn.val; break;
            case DT_RELRSZ: m.relr_sz_ = dn.val; break;
            case DT_INIT: m.init_fn_ = dn.val; break;
            case DT_FINI: m.fini_fn_ = dn.val; break;
            case DT_INIT_ARRAY: m.init_arr_ = dn.val; break;
            case DT_INIT_ARRAYSZ: m.init_arr_sz_ = dn.val; break;
            case DT_FINI_ARRAY: m.fini_arr_ = dn.val; break;
            case DT_FINI_ARRAYSZ: m.fini_arr_sz_ = dn.val; break;
            default: break;
        }
    }
    if (syment != sizeof(Sym)) return fail(err, "DT_SYMENT %llu is not %zu", (unsigned long long)syment, sizeof(Sym));

    // Number of dynamic symbols: section headers when present, else DT_HASH, else DT_GNU_HASH, else the gap to the string table.
    uint64_t nsyms = 0;
    if (symtab) {
        if (eh.shoff && eh.shnum && eh.shentsize == sizeof(Shdr) && eh.shoff <= n && uint64_t(eh.shnum) * sizeof(Shdr) <= n - eh.shoff) {
            for (unsigned i = 0; i < eh.shnum; i++) {
                Shdr sh;
                memcpy(&sh, d + eh.shoff + i * sizeof(Shdr), sizeof sh);
                if (sh.type == SHT_DYNSYM && sh.entsize == sizeof(Sym)) nsyms = sh.size / sizeof(Sym);
            }
        }
        if (!nsyms && hash && in_image(hash, 8)) {
            uint32_t nchain;
            memcpy(&nchain, at(hash) + 4, 4);
            nsyms = nchain;
        }
        if (!nsyms && gnu_hash && in_image(gnu_hash, 16)) {
            uint32_t h[4];
            memcpy(h, at(gnu_hash), 16);
            uint32_t nbuckets = h[0], symoffset = h[1], bloom = h[2];
            uint64_t buckets = gnu_hash + 16 + uint64_t(bloom) * 8, chains = buckets + uint64_t(nbuckets) * 4;
            if (in_image(buckets, uint64_t(nbuckets) * 4)) {
                uint32_t last = 0;
                for (uint32_t i = 0; i < nbuckets; i++) {
                    uint32_t b;
                    memcpy(&b, at(buckets) + i * 4, 4);
                    last = std::max(last, b);
                }
                if (last >= symoffset) {
                    for (;;) {       // walk the chain of the highest bucket to its end marker
                        uint64_t ea = chains + uint64_t(last - symoffset) * 4;
                        if (!in_image(ea, 4)) break;
                        uint32_t e;
                        memcpy(&e, at(ea), 4);
                        last++;
                        if (e & 1) break;
                    }
                    nsyms = last;
                } else {
                    nsyms = symoffset;
                }
            }
        }
        if (!nsyms && strtab > symtab) nsyms = (strtab - symtab) / sizeof(Sym);
        if (nsyms > (1u << 24) || !in_image(symtab, nsyms * sizeof(Sym))) return fail(err, "dynamic symbol table (%llu entries at 0x%llx) is outside the image", (unsigned long long)nsyms, (unsigned long long)symtab);
        if (!strsz) strsz = hi - strtab;
        if (!strtab || !in_image(strtab, strsz)) return fail(err, "dynamic string table is outside the image");
        m.symbols_.reserve(nsyms);
        for (uint64_t i = 0; i < nsyms; i++) {
            Sym s;
            memcpy(&s, at(symtab) + i * sizeof(Sym), sizeof s);
            ModuleSymbol ms;
            if (s.name < strsz) {
                const char* p = reinterpret_cast<const char*>(at(strtab)) + s.name;
                size_t maxlen = static_cast<size_t>(strsz - s.name);
                ms.name.assign(p, strnlen(p, maxlen));
            }
            ms.value = s.value;
            ms.size = s.size;
            ms.type = s.info & 0xf;
            ms.bind = s.info >> 4;
            ms.shndx = s.shndx;
            m.symbols_.push_back(std::move(ms));
        }
    }
    return true;
}

// ---- phase 2: link ---------------------------------------------------------------------------------------------------

// Absolute address a relocation against symbol `symidx` of module m stands for.
uint64_t LoaderAccess::resolve(Module& m, const std::vector<elf::Sym>&, uint32_t symidx, bool* weak_null) {
    *weak_null = false;
    if (symidx == 0) return 0;
    if (symidx >= m.symbols_.size()) { NXLOG_W(kTag, "%s: relocation names symbol %u beyond the table (%zu)", m.name_.c_str(), symidx, m.symbols_.size()); return 0; }
    const ModuleSymbol& s = m.symbols_[symidx];
    RelocStats& st = m.stats_;
    if (!s.name.empty() && g_overrides.count(s.name)) {
        st.to_hle++;
        return hle_address(s.name.c_str());
    }
    if (s.defined()) {
        st.to_own++;
        if (s.type == 6) return s.value;                                   // TLS symbol: offset inside the TLS segment
        return s.shndx == elf::SHN_ABS ? s.value : m.bias_ + s.value;
    }
    // The linker symbols the SDK's start-up code imports: the module's own relocation tables (Horizon's rtld defines them per module).
    if (s.name.size() > 6 && s.name.compare(0, 6, "__rel_") == 0) {
        if (s.name == "__rel_dyn_start") { st.to_own++; return m.bias_ + m.rela_; }
        if (s.name == "__rel_dyn_end") { st.to_own++; return m.bias_ + m.rela_ + m.rela_sz_; }
        if (s.name == "__rel_plt_start") { st.to_own++; return m.bias_ + m.jmprel_; }
        if (s.name == "__rel_plt_end") { st.to_own++; return m.bias_ + m.jmprel_ + m.jmprel_sz_; }
    }
    // Undefined: another module, host data, HLE handler, weak null, logging stub - in that order.
    for (auto& o : g_modules) {
        if (o.get() == &m) continue;
        if (uint64_t a = o->find_symbol(s.name.c_str())) { st.to_other_module++; return a; }
    }
    auto d = g_data.find(s.name);
    if (d != g_data.end()) { st.to_host_data++; return d->second; }
    if (hle_lookup(s.name.c_str())) { st.to_hle++; return hle_address(s.name.c_str()); }
    if (s.bind == 2) { st.to_weak_null++; *weak_null = true; return 0; }
    st.to_stub++;
    NXLOG_W(kTag, "%s: import %s has no host implementation: bound to a logging stub", m.name_.c_str(), s.name.c_str());
    return hle_address(s.name.c_str());
}

bool LoaderAccess::link(Module& m, bool guest_relinks) {
    using namespace elf;
    if (m.linked_) return true;
    const uint64_t lo = m.base_ - m.bias_, hi = lo + m.size_;
    std::vector<Sym> unused;
    bool ok = true;
    // guest_relinks: every slot written here, with the image's value, to put back once the module is checked (see below)
    std::vector<std::pair<uint64_t, uint64_t>> written;
    const uint64_t tables[2][2] = {{m.rela_, m.rela_sz_}, {m.jmprel_, m.jmprel_sz_}};
    for (auto& t : tables) {
        if (!t[0] || !t[1]) continue;
        if (t[0] < lo || t[0] > hi || t[1] > hi - t[0]) { NXLOG_E(kTag, "%s: relocation table is outside the image", m.name_.c_str()); ok = false; continue; }
        for (uint64_t i = 0; i + sizeof(Rela) <= t[1]; i += sizeof(Rela)) {
            Rela r;
            memcpy(&r, reinterpret_cast<const void*>(m.bias_ + t[0] + i), sizeof r);
            m.stats_.total++;
            const uint32_t type = static_cast<uint32_t>(r.info), symidx = static_cast<uint32_t>(r.info >> 32);
            if (r.offset < lo || r.offset + 8 > hi) { m.stats_.out_of_range++; NXLOG_W(kTag, "%s: relocation at 0x%llx is outside the image", m.name_.c_str(), (unsigned long long)r.offset); continue; }
            uint64_t* slot = reinterpret_cast<uint64_t*>(m.bias_ + r.offset);
            if (guest_relinks) written.emplace_back(r.offset, *slot);
            bool weak_null = false;
            switch (type) {
                case R_AARCH64_RELATIVE:
                    m.stats_.relative++;
                    *slot = m.bias_ + static_cast<uint64_t>(r.addend);
                    break;
                case R_AARCH64_ABS64:
                case R_AARCH64_GLOB_DAT:
                case R_AARCH64_JUMP_SLOT: {
                    (type == R_AARCH64_ABS64 ? m.stats_.abs64 : type == R_AARCH64_GLOB_DAT ? m.stats_.glob_dat : m.stats_.jump_slot)++;
                    uint64_t s = resolve(m, unused, symidx, &weak_null);
                    *slot = weak_null ? 0 : s + static_cast<uint64_t>(r.addend);
                    break;
                }
                case R_AARCH64_TLS_TPREL64: {
                    m.stats_.tls++;
                    uint64_t s = symidx && symidx < m.symbols_.size() && m.symbols_[symidx].defined() ? m.symbols_[symidx].value : 0;
                    *slot = s + static_cast<uint64_t>(r.addend) + m.tls_.tp_offset;
                    break;
                }
                case R_AARCH64_TLS_DTPMOD64:
                    m.stats_.tls++;
                    *slot = 1;
                    break;
                case R_AARCH64_TLS_DTPREL64: {
                    m.stats_.tls++;
                    uint64_t s = symidx && symidx < m.symbols_.size() && m.symbols_[symidx].defined() ? m.symbols_[symidx].value : 0;
                    *slot = s + static_cast<uint64_t>(r.addend);
                    break;
                }
                default:
                    m.stats_.unhandled++;
                    if (m.stats_.unhandled <= 8) NXLOG_W(kTag, "%s: relocation type %u (sym %u) at 0x%llx is not handled", m.name_.c_str(), type, symidx, (unsigned long long)r.offset);
                    break;
            }
        }
    }
    // DT_RELR: packed RELATIVE relocations (the addend is the value already in the slot). An even entry is the address of
    // one slot; an odd entry is a bitmap over the 63 slots after the last one relocated.
    if (m.relr_ && m.relr_sz_) {
        if (m.relr_ < lo || m.relr_ > hi || m.relr_sz_ > hi - m.relr_) {
            NXLOG_E(kTag, "%s: DT_RELR table is outside the image", m.name_.c_str());
            ok = false;
        } else {
            auto fix = [&](uint64_t off) {
                m.stats_.total++;
                if (off < lo || off + 8 > hi) { m.stats_.out_of_range++; return; }
                m.stats_.relative++;
                uint64_t* slot = reinterpret_cast<uint64_t*>(m.bias_ + off);
                if (guest_relinks) written.emplace_back(off, *slot);
                *slot += m.bias_;
            };
            uint64_t where = 0;
            for (uint64_t i = 0; i + 8 <= m.relr_sz_; i += 8) {
                const uint64_t e = *reinterpret_cast<const uint64_t*>(m.bias_ + m.relr_ + i);
                if (!(e & 1)) {
                    fix(e);
                    where = e + 8;
                } else {
                    for (int bit = 1; bit < 64; bit++)
                        if (e >> bit & 1) fix(where + uint64_t(bit - 1) * 8);
                    where += 63 * 8;
                }
            }
        }
    }
    // Init / fini arrays are read after relocation (their slots were filled by RELATIVE relocations).
    auto read_array = [&](uint64_t vaddr, uint64_t bytes, std::vector<uint64_t>& out) {
        if (!vaddr || !bytes) return;
        if (vaddr < lo || vaddr > hi || bytes > hi - vaddr) { NXLOG_E(kTag, "%s: init/fini array is outside the image", m.name_.c_str()); ok = false; return; }
        for (uint64_t i = 0; i + 8 <= bytes; i += 8) out.push_back(*reinterpret_cast<const uint64_t*>(m.bias_ + vaddr + i));
    };
    m.init_.clear();
    m.fini_.clear();
    read_array(m.init_arr_, m.init_arr_sz_, m.init_);
    read_array(m.fini_arr_, m.fini_arr_sz_, m.fini_);
    // The guest's own dynamic linker relocates this module again from the image's values, and some of its passes are not
    // idempotent (DT_RELR and the lazy JUMP_SLOT pass add the base to what is in the slot; rtld aborts when a JUMP_SLOT does not
    // hold the image's PLT value). Every slot goes back to the image's value, newest write first; the checks, statistics and the
    // init/fini arrays above keep the host's result.
    if (guest_relinks && !written.empty()) {
        for (auto it = written.rbegin(); it != written.rend(); ++it) *reinterpret_cast<uint64_t*>(m.bias_ + it->first) = it->second;
        NXLOG_I(kTag, "%s: %zu relocated slots restored to the image's values for the guest's dynamic linker", m.name_.c_str(), written.size());
    }
    if (m.tls_.present() && !g_tls.present()) g_tls = m.tls_;
    protect(m);
    m.linked_ = true;
    const RelocStats& s = m.stats_;
    NXLOG_I(kTag, "linked %s: %zu relocations (relative %zu, abs64 %zu, glob_dat %zu, jump_slot %zu, tls %zu, unhandled %zu); targets: own %zu, other module %zu, host data %zu, hle %zu, weak null %zu, stub %zu",
            m.name_.c_str(), s.total, s.relative, s.abs64, s.glob_dat, s.jump_slot, s.tls, s.unhandled, s.to_own, s.to_other_module, s.to_host_data, s.to_hle, s.to_weak_null, s.to_stub);
    return ok;
}

// Page protections from the segment flags (union per page), then RELRO pages become read-only.
void LoaderAccess::protect(Module& m) {
    using namespace elf;
    const uint64_t lo = m.base_ - m.bias_;
    const size_t pages = static_cast<size_t>(m.size_ / mem::kPage);
    std::vector<uint8_t> perm(pages, 0);
    std::vector<uint8_t> has(pages, 0);
    for (auto& s : m.segs_) {
        if (!s.memsz) continue;
        uint64_t first = (align_down(s.vaddr, mem::kPage) - lo) / mem::kPage, last = (align_up(s.vaddr + s.memsz, mem::kPage) - lo) / mem::kPage;
        for (uint64_t p = first; p < last && p < pages; p++) { perm[p] |= static_cast<uint8_t>(s.flags & 7); has[p] = 1; }
    }
    if (m.relro_size_) {   // whole pages inside RELRO lose write permission once relocations are done
        uint64_t first = (align_up(m.relro_start_, mem::kPage) - lo) / mem::kPage, last = (align_down(m.relro_start_ + m.relro_size_, mem::kPage) - lo) / mem::kPage;
        for (uint64_t p = first; p < last && p < pages; p++) perm[p] &= ~static_cast<uint8_t>(PF_W);
    }
    for (size_t p = 0; p < pages;) {
        size_t q = p;
        while (q < pages && perm[q] == perm[p] && has[q] == has[p]) q++;
        uint8_t f = perm[p];
        mem::Prot pr = !has[p] || !(f & (PF_R | PF_W | PF_X)) ? mem::Prot::None
                       : (f & PF_X) ? ((f & PF_W) ? mem::Prot::RWX : mem::Prot::RX)
                       : (f & PF_W) ? mem::Prot::RW : mem::Prot::R;
        uint64_t addr = m.base_ + p * mem::kPage, len = (q - p) * mem::kPage;
        mem::protect(addr, len, pr);
        // svcQueryMemory reports Horizon permission bits (R = 1, W = 2, X = 4), not the ELF ones (X = 1, W = 2, R = 4).
        const uint32_t hp = ((f & PF_R) ? 1u : 0u) | ((f & PF_W) ? 2u : 0u) | ((f & PF_X) ? 4u : 0u);
        if (has[p]) mem::vm_set(addr, len, (f & PF_W) ? mem::kMemCodeData : mem::kMemCode, hp);
        p = q;
    }
}

// ---- public API --------------------------------------------------------------------------------------------------------

Module* module_load_memory(const void* data, size_t size, const LoadOptions& opt, std::string* err) {
    std::lock_guard<std::recursive_mutex> lk(g_lock);
    return LoaderAccess::create_and_map(static_cast<const uint8_t*>(data), size, opt, err);
}

Module* module_load_file(const char* path, const LoadOptions& opt_in, std::string* err) {
    std::ifstream f(path, std::ios::binary | std::ios::ate);
    if (!f) { fail(err, "cannot open %s", path); return nullptr; }
    std::streamsize sz = f.tellg();
    if (sz <= 0 || static_cast<uint64_t>(sz) > (1ull << 31)) { fail(err, "%s has an unreasonable size (%lld)", path, static_cast<long long>(sz)); return nullptr; }
    std::vector<uint8_t> bytes(static_cast<size_t>(sz));
    f.seekg(0);
    f.read(reinterpret_cast<char*>(bytes.data()), sz);
    if (!f) { fail(err, "short read on %s", path); return nullptr; }
    LoadOptions opt = opt_in;
    if (opt.name.empty()) {
        const char* slash = strrchr(path, '\\');
        const char* slash2 = strrchr(path, '/');
        if (slash2 > slash) slash = slash2;
        opt.name = slash ? slash + 1 : path;
    }
    return module_load_memory(bytes.data(), bytes.size(), opt, err);
}

bool module_link_all() {
    std::lock_guard<std::recursive_mutex> lk(g_lock);
    // Horizon's dynamic linker (the ExeFS module "rtld", entered first) relocates itself and every other module when the process
    // starts. The rtld of SDK 20-era titles (itself built with DT_RELR) relocates from the image's values and its passes add to
    // the slots, so after the host has linked and checked the modules the images go back to the NSO's values for it. An older
    // rtld (no DT_RELR, RELA only) is idempotent and keeps the host's result.
    bool guest_relinks = false;
    for (auto& m : g_modules) guest_relinks = guest_relinks || (m->name() == "rtld" && LoaderAccess::has_relr(*m));
    bool ok = true;
    for (auto& m : g_modules) ok = LoaderAccess::link(*m, guest_relinks) && ok;
    return ok;
}

bool module_protect_all() {
    std::lock_guard<std::recursive_mutex> lk(g_lock);
    for (auto& m : g_modules)
        if (!m->linked()) LoaderAccess::protect(*m);
    return true;
}

size_t module_register_functions(Module* m, const FuncEntry* table, size_t count) {
    size_t ok = 0, bad = 0;
    for (size_t i = 0; i < count; i++) {
        if (!m->contains(table[i].addr) || !table[i].fn) { bad++; continue; }
        fn_register(table[i].addr, table[i].fn);
        ok++;
    }
    if (bad) NXLOG_W(kTag, "%s: %zu of %zu function entries are outside the image or null: rejected", m->name().c_str(), bad, count);
    NXLOG_D(kTag, "%s: %zu lifted functions registered", m->name().c_str(), ok);
    return ok;
}

size_t module_register_functions(Module* m, const ::FnEntry* table, size_t count) {
    static_assert(sizeof(::FnEntry) == sizeof(FuncEntry), "FnEntry and FuncEntry must have the same layout");
    return module_register_functions(m, reinterpret_cast<const FuncEntry*>(table), count);
}

namespace {
bool run_one(Module* m, ::Ctx* ctx, uint64_t addr, const char* what) {
    if (!addr || addr == ~0ull) return true;
    if (!fn_lookup(addr) && !hle_is_thunk(addr)) {
        NXLOG_E(kTag, "%s: %s 0x%llx is not a registered function: skipped", m->name().c_str(), what, (unsigned long long)addr);
        return false;
    }
    dispatch_call(ctx, addr);
    return true;
}
}  // namespace

bool module_run_init(Module* m, ::Ctx* ctx) {
    if (!m->linked()) { NXLOG_E(kTag, "%s: init before link", m->name().c_str()); return false; }
    bool ok = run_one(m, ctx, m->init_function(), "DT_INIT");
    for (uint64_t a : m->init_array()) ok = run_one(m, ctx, a, "init_array entry") && ok;
    return ok;
}

bool module_run_fini(Module* m, ::Ctx* ctx) {
    bool ok = true;
    for (size_t i = m->fini_array().size(); i-- > 0;) ok = run_one(m, ctx, m->fini_array()[i], "fini_array entry") && ok;
    ok = run_one(m, ctx, m->fini_function(), "DT_FINI") && ok;
    return ok;
}

Module* module_at(uint64_t addr) {
    std::lock_guard<std::recursive_mutex> lk(g_lock);
    for (auto& m : g_modules)
        if (m->contains(addr)) return m.get();
    return nullptr;
}

Module* module_by_name(const char* name) {
    std::lock_guard<std::recursive_mutex> lk(g_lock);
    for (auto& m : g_modules)
        if (m->name() == name) return m.get();
    return nullptr;
}

std::vector<Module*> modules() {
    std::lock_guard<std::recursive_mutex> lk(g_lock);
    std::vector<Module*> v;
    for (auto& m : g_modules) v.push_back(m.get());
    return v;
}

uint64_t module_find_symbol(const char* sym) {
    std::lock_guard<std::recursive_mutex> lk(g_lock);
    for (auto& m : g_modules)
        if (uint64_t a = m->find_symbol(sym)) return a;
    return 0;
}

std::string module_describe(uint64_t addr) {
    char buf[384];
    if (hle_is_thunk(addr)) {
        snprintf(buf, sizeof buf, "hle:%s", hle_name_at(addr));
        return buf;
    }
    if (Module* m = module_at(addr)) {
        std::string s = m->describe(addr);
        if (s.empty()) snprintf(buf, sizeof buf, "%s+0x%llx", m->name().c_str(), (unsigned long long)(addr - m->base()));
        else snprintf(buf, sizeof buf, "%s+0x%llx (%s)", m->name().c_str(), (unsigned long long)(addr - m->base()), s.c_str());
        return buf;
    }
    snprintf(buf, sizeof buf, "0x%llx", (unsigned long long)addr);
    return buf;
}

bool module_symbol_overridden(const char* sym) {
    std::lock_guard<std::recursive_mutex> lk(g_lock);
    return g_overrides.count(sym) != 0;
}

void module_provide_data(const char* sym, uint64_t addr) {
    std::lock_guard<std::recursive_mutex> lk(g_lock);
    g_data[sym] = addr;
}

void module_override_symbol(const char* sym) {
    {
        std::lock_guard<std::recursive_mutex> lk(g_lock);
        g_overrides.insert(sym);
    }
    import_cache_invalidate();       // an import resolved before the override must be resolved again
}

const TlsInfo& module_tls_template() { return g_tls; }

void modules_unload_all() {
    std::lock_guard<std::recursive_mutex> lk(g_lock);
    for (auto& m : g_modules) mem::release_fixed(m->base());
    g_modules.clear();
    g_data.clear();
    g_overrides.clear();
    g_next_base = 0;
    g_tls = {};
    fn_clear();
    nxmodules_clear();
}

}  // namespace nxrt
