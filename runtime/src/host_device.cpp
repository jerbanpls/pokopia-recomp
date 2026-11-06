// SPDX-License-Identifier: GPL-3.0-or-later
// The machine the runtime runs on (include/nxrt/host_device.h). Based on the Steam Deck detection in tinfoil-chewer's Linux fork
// (tinfoil-chewer/pokopia-recomp-linux 8b150a4), widened to the Legion Go S and to Windows handhelds.
#include "nxrt/host_device.h"

#include "nxrt/config.h"
#include "nxrt/log.h"

#include <windows.h>

#include <algorithm>
#include <cctype>
#include <fstream>
#include <string>

namespace nxrt {

namespace {

std::string lower_trimmed(std::string s) {
    while (!s.empty() && std::isspace(static_cast<unsigned char>(s.back()))) s.pop_back();
    size_t i = 0;
    while (i < s.size() && std::isspace(static_cast<unsigned char>(s[i]))) ++i;
    s.erase(0, i);
    std::transform(s.begin(), s.end(), s.begin(), [](unsigned char c) { return static_cast<char>(std::tolower(c)); });
    return s;
}

// An SMBIOS string: HKLM\HARDWARE\DESCRIPTION\System\BIOS (Windows; Wine's wineboot fills it from SMBIOS), else the Linux DMI file
// through Wine's Z: drive (on Windows that path does not exist).
std::string smbios(const char* value, const char* dmi_file) {
    char buf[128] = {};
    DWORD n = sizeof buf;
    if (RegGetValueA(HKEY_LOCAL_MACHINE, "HARDWARE\\DESCRIPTION\\System\\BIOS", value, RRF_RT_REG_SZ, nullptr, buf, &n) == ERROR_SUCCESS)
        return lower_trimmed(buf);
    std::ifstream f(std::string("Z:\\sys\\class\\dmi\\id\\") + dmi_file);
    std::string s;
    if (f && std::getline(f, s)) return lower_trimmed(s);
    return {};
}

bool detect(std::string* why) {
    const std::string forced = lower_trimmed(config_get_str("host", "handheld", "auto"));
    if (forced == "1" || forced == "on" || forced == "true" || forced == "yes") return *why = "[host] handheld = on", true;
    if (forced == "0" || forced == "off" || forced == "false" || forced == "no") return *why = "[host] handheld = off", false;
    char env[16] = {};
    const DWORD n = GetEnvironmentVariableA("SteamDeck", env, sizeof env);
    if (n > 0 && n < sizeof env && lower_trimmed(std::string(env, n)) == "1") return *why = "Steam's SteamDeck=1", true;
    const std::string product = smbios("SystemProductName", "product_name"), family = smbios("SystemFamily", "product_family");
    // Steam Deck LCD / OLED. The Go S's product names are SKU codes (83N6, ...), its family names the model: "Legion Go S 8APU1".
    if (product == "jupiter" || product == "galileo" || family.rfind("legion go s", 0) == 0) return *why = "board " + product + " / " + family, true;
    *why = "not a known handheld (board '" + product + "' / '" + family + "')";
    return false;
}

}  // namespace

bool host_is_handheld() {
    static const bool handheld = [] {
        std::string why;
        const bool h = detect(&why);
        NXLOG_I("host", "handheld: %s (%s)", h ? "yes" : "no", why.c_str());
        return h;
    }();
    return handheld;
}

bool host_is_steam_deck() {
    static const bool deck = [] {
        char env[16] = {};
        const DWORD n = GetEnvironmentVariableA("SteamDeck", env, sizeof env);
        const std::string product = smbios("SystemProductName", "product_name");
        const bool d = (n > 0 && n < sizeof env && lower_trimmed(std::string(env, n)) == "1") || product == "jupiter" || product == "galileo";
        NXLOG_I("host", "steam deck: %s (board '%s')", d ? "yes" : "no", product.c_str());
        return d;
    }();
    return deck;
}

bool host_is_wine() {
    static const bool wine = [] {
        HMODULE nt = GetModuleHandleW(L"ntdll.dll");
        return nt && GetProcAddress(nt, "wine_get_version");
    }();
    return wine;
}

}  // namespace nxrt
