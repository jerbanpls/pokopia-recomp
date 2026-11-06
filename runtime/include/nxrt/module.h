// SPDX-License-Identifier: GPL-3.0-or-later
// Module loader and linker: maps an AArch64 ELF64 image (what nx2elf produces from an NSO, or any ET_DYN/ET_EXEC AArch64 ELF)
// at its base address, applies its dynamic relocations, resolves imports (other modules -> host data -> HLE handlers -> weak null
// -> logging stub), registers lifted functions in the call_indirect table and runs the init array.
// Pattern: smo-recomp runtime/core/hostrt.c runtime_init (identity mapping, relocation types, protection), SuperMarioWonderRecomp
// runtime/src/loader.cpp (two-phase map-then-link, import thunks).
//
// Two phases so that symbols of every module are known before relocating:
//   Module* a = module_load_file(...);  Module* b = module_load_file(...);   // map + parse
//   module_link_all();                                                         // relocate, protect
//   module_register_functions(a, table, n);  module_run_init(a, ctx);          // lifted code + constructors
#pragma once
#include <cstddef>
#include <cstdint>
#include <mutex>
#include <string>
#include <unordered_map>
#include <vector>

#include "nxrt/abi.h"

namespace nxrt {

struct ModuleSymbol {
    std::string name;
    uint64_t value = 0;        // module-relative (st_value)
    uint64_t size = 0;
    uint8_t type = 0;          // STT_*: 0 notype, 1 object, 2 func, 6 tls
    uint8_t bind = 0;          // STB_*: 0 local, 1 global, 2 weak
    uint16_t shndx = 0;        // 0 = undefined (an import)
    bool defined() const { return shndx != 0; }
};

struct TlsInfo {
    uint64_t init_addr = 0;    // absolute address of the initial image (.tdata)
    uint64_t file_size = 0;
    uint64_t mem_size = 0;     // .tdata + .tbss
    uint64_t align = 1;
    uint64_t tp_offset = 0;    // offset of the block from the thread pointer (AArch64 variant I: 16 rounded up to align)
    bool present() const { return mem_size != 0; }
};

struct RelocStats {
    size_t total = 0;
    size_t relative = 0, abs64 = 0, glob_dat = 0, jump_slot = 0, tls = 0, irelative = 0;
    size_t unhandled = 0;           // relocation types we do not apply (logged)
    size_t to_own = 0;              // against a symbol this module defines
    size_t to_other_module = 0;     // resolved to another module's export
    size_t to_host_data = 0;        // resolved to memory the host provided (module_provide_data)
    size_t to_hle = 0;              // resolved to an HLE handler thunk
    size_t to_weak_null = 0;        // undefined weak symbol: null
    size_t to_stub = 0;             // unresolved: a logging stub thunk
    size_t out_of_range = 0;        // relocation target outside the image (skipped)
};

struct FuncEntry {                  // one lifted function: absolute guest address and its host implementation
    uint64_t addr;
    GuestFn fn;
};

struct LoadOptions {
    std::string name;               // defaults to the file name
    uint64_t base = 0;              // load address; 0 = mem::kMainBase for the first module, then the next 64 KiB boundary after the previous one
    uint64_t max_image_size = 0x40000000ull;   // refuse images bigger than this (1 GiB)
    bool protect = true;            // apply segment protections after linking
};

class Module {
public:
    const std::string& name() const { return name_; }
    uint64_t base() const { return base_; }
    uint64_t size() const { return size_; }                   // page-rounded span of the image
    uint64_t entry() const { return entry_; }                 // absolute address of e_entry, 0 if none
    bool linked() const { return linked_; }
    bool contains(uint64_t addr) const { return addr - base_ < size_; }
    const std::vector<ModuleSymbol>& symbols() const { return symbols_; }
    uint64_t find_symbol(const char* sym) const;              // absolute address of a defined symbol or 0
    const std::vector<uint64_t>& init_array() const { return init_; }   // absolute addresses, after linking
    const std::vector<uint64_t>& fini_array() const { return fini_; }
    uint64_t init_function() const { return init_fn_ ? bias_ + init_fn_ : 0; }   // DT_INIT / DT_FINI, absolute, 0 if none
    uint64_t fini_function() const { return fini_fn_ ? bias_ + fini_fn_ : 0; }
    const TlsInfo& tls() const { return tls_; }
    const RelocStats& stats() const { return stats_; }
    // Names the nearest preceding symbol: "name+0x10" or empty.
    std::string describe(uint64_t addr) const;
    // The NSO header's 32-byte module id (the build id) as 64 lowercase hex digits; empty for a module not loaded from an NSO. Code that
    // hooks one build of a module checks it (game modules: hook sites are offsets of one build).
    const std::string& build_id() const { return build_id_; }
    void set_build_id(const uint8_t (&id)[32]);

private:
    friend struct LoaderAccess;
    std::string name_;
    std::string build_id_;
    uint64_t base_ = 0, size_ = 0, entry_ = 0;
    uint64_t bias_ = 0;        // added to every ELF virtual address (ET_DYN: the load base; ET_EXEC: 0)
    bool linked_ = false;
    std::vector<ModuleSymbol> symbols_;
    std::vector<uint64_t> init_, fini_;
    TlsInfo tls_;
    RelocStats stats_;
    struct Seg { uint64_t vaddr, memsz, filesz, offset; uint32_t flags; };
    std::vector<Seg> segs_;
    uint64_t relro_start_ = 0, relro_size_ = 0;
    uint64_t rela_ = 0, rela_sz_ = 0, jmprel_ = 0, jmprel_sz_ = 0;     // vaddrs from the dynamic section
    uint64_t relr_ = 0, relr_sz_ = 0;                                 // DT_RELR (SDK 20 modules: packed RELATIVE relocations)
    std::vector<uint64_t> init_rel_, fini_rel_;                       // module-relative init/fini entries read before relocation
    uint64_t init_arr_ = 0, init_arr_sz_ = 0, fini_arr_ = 0, fini_arr_sz_ = 0;
    uint64_t init_fn_ = 0, fini_fn_ = 0;                              // DT_INIT / DT_FINI
    mutable std::once_flag index_once_;                               // name -> first global/weak definition, built on the first find_symbol
    mutable std::unordered_map<std::string, uint32_t> index_;
};

// Phase 1: map and parse. Returns null (with *err) on a malformed image, an occupied address range or an oversize image.
Module* module_load_file(const char* path, const LoadOptions& opt = {}, std::string* err = nullptr);
Module* module_load_memory(const void* data, size_t size, const LoadOptions& opt = {}, std::string* err = nullptr);
// An NSO (the ExeFS container of main, rtld, sdk, subsdk*): LZ4 segments are expanded and mapped like an ELF module at opt.base
// (src/loader/nso.cpp). The module's addresses stay module relative, exactly as in the lifter's input.
Module* module_load_nso_file(const char* path, const LoadOptions& opt = {}, std::string* err = nullptr);
Module* module_load_nso_memory(const void* data, size_t size, const LoadOptions& opt = {}, std::string* err = nullptr);
// Phase 2: apply relocations of every module that is mapped but not yet linked, then protect segments. False when a module failed.
// When the modules include an SDK 20-era "rtld" (Horizon's dynamic linker, built with DT_RELR; it relocates itself and every module
// again at start-up from the image's values, adding to the slots), every relocated slot is put back to the image's value after the
// module is linked and checked (statistics and init/fini arrays keep the host's result), so rtld's passes are applied once.
bool module_link_all();
// For a title that carries its own dynamic linker (Horizon's rtld relocates every module itself): applies only the page protections and
// the Horizon memory-state records (svcQueryMemory sees the modules as code / code data), leaving the images exactly as the NSO has
// them. Do not combine with module_link_all() on the same modules.
bool module_protect_all();
// Lifted code: registers each entry in the call_indirect table. Entries outside the module's image are rejected (counted, logged).
size_t module_register_functions(Module* m, const FuncEntry* table, size_t count);
size_t module_register_functions(Module* m, const ::FnEntry* table, size_t count);   // the lifter's FnEntry (same layout)
// Registers a lifted module's tables (NxModule from nxrecomp/ctx.h, ABI 2) with the ABI version this runtime implements: its functions
// go to the call table and its tables to the registry (nxmodule.h: import resolution, unwinder). Returns the number of functions
// registered, 0 when the module is refused. `m` is the mapped image (null: the NxModule's own base/text_end range is trusted).
// nxmodule_register() reports why a module was refused.
size_t module_register_nxmodule(Module* m, const NxModule* nx);
// Runs DT_INIT then every init_array entry on `ctx` through dispatch_call (entries must be registered functions or HLE thunks).
bool module_run_init(Module* m, ::Ctx* ctx);
bool module_run_fini(Module* m, ::Ctx* ctx);       // fini_array in reverse order, then DT_FINI

// Lookup across modules.
Module* module_at(uint64_t addr);
Module* module_by_name(const char* name);
std::vector<Module*> modules();
uint64_t module_find_symbol(const char* sym);      // first module (load order) defining it (global before weak), absolute address or 0
std::string module_describe(uint64_t addr);        // "name+0xoff (symbol+0x10)" / "hle:name" / hex
// Forces every relocation against `sym` (defined anywhere or not) to the HLE thunk of that name: the replacement of a recompiled
// function by host code ("nn:: import replacement where hot"). Register the handler with hle_register first. Set before module_link_all.
void module_override_symbol(const char* sym);
bool module_symbol_overridden(const char* sym);    // was module_override_symbol(sym) called
// Host-provided data for imported data symbols: the symbol resolves to `addr` (set before module_link_all).
void module_provide_data(const char* sym, uint64_t addr);
// The TLS template of the first loaded module with a PT_TLS segment; threads copy it into their TLS block (kernel.h).
const TlsInfo& module_tls_template();

// Tests: unmap and forget every module (also clears the function table registrations made through module_register_functions).
void modules_unload_all();

}  // namespace nxrt
