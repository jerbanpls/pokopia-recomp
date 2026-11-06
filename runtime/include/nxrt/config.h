// SPDX-License-Identifier: GPL-3.0-or-later
// Configuration: an ini file plus environment-variable switches.
//   [section]
//   key = value        ; comment    # comment
// Lookup order for config_get_*("diag", "watchdog_seconds", def):
//   1. environment variable NXRT_DIAG_WATCHDOG_SECONDS   (NXRT_ + upper-case section + '_' + upper-case key)
//   2. the ini value
//   3. the default
// Keys outside any section use the section name "" and the env name NXRT_<KEY>.
#pragma once
#include <cstdint>
#include <string>

namespace nxrt {

// Loads (replaces) the ini from a file. Returns false when the file cannot be read (the config stays empty).
bool config_load_file(const char* path);
// Loads from a string (tests). Replaces the current ini content.
void config_load_string(const char* text);
void config_clear();
// Programmatic override, as if it were in the ini (tests, command-line options). Env still wins over it.
void config_set(const char* section, const char* key, const char* value);

std::string config_get_str(const char* section, const char* key, const char* def = "");
int64_t config_get_int(const char* section, const char* key, int64_t def = 0);      // decimal, 0x hex, optional k/m/g suffix not supported
double config_get_double(const char* section, const char* key, double def = 0.0);
bool config_get_bool(const char* section, const char* key, bool def = false);       // 1/0 true/false yes/no on/off, case-insensitive
bool config_has(const char* section, const char* key);                              // set by env or ini

// Where a value came from: "env", "ini" or "default". For the startup dump.
const char* config_source(const char* section, const char* key);
// Logs (Info, tag "config") every key read so far with its effective value and source.
void config_log_effective();
// Caches lookups per thread from now on (on = true) or stops caching. The caller promises that the process environment does not
// change any more (the cache cannot see _putenv); ini changes through this API are seen at once. [config] cache = false refuses.
// Returns whether the cache is on. nxrt::init turns it on when InitOptions::cache_env is set (the game exe).
bool config_cache_env(bool on);
// The environment variable name used for a key, e.g. NXRT_DIAG_WATCHDOG_SECONDS.
std::string config_env_name(const char* section, const char* key);

}  // namespace nxrt
