// SPDX-License-Identifier: GPL-3.0-or-later
// ldn: ldn:u (IUserServiceCreator -> IUserLocalCommunicationService), Nintendo local wireless, carried by a vnet::Lobby (multiplayer
// phase 2): a network the game creates is a room on the relay server, a scan lists the server's rooms, a connect joins one, and while the
// node is in a room its lobby link is the current vnet link, so the bsd UDP sockets reach the other nodes (169.254.R.N/24).
// Game-independent.
//
// Adapted from Ryujinx (MIT): src/Ryujinx.HLE/HOS/Services/Ldn/Types/* (NetworkInfo, CommonNetworkInfo, LdnNetworkInfo, NodeInfo,
//   NodeLatestUpdate, ScanFilter, NetworkConfig, SecurityConfig, SecurityParameter, UserConfig, IntentId, NetworkId), Ldn/ResultCode.cs,
//   Ldn/UserServiceCreator/IUserLocalCommunicationService.cs (raw argument layouts: CreateNetwork reads SecurityConfig, UserConfig, 4 pad,
//   NetworkConfig; Connect reads SecurityConfig, UserConfig, u32 version, u32 option with the NetworkInfo in buffer 0x19; Scan reads the
//   channel as 8 bytes, then the ScanFilter) and the DisconnectReason enum.
// Switchbrew "LDN services": the command table (Switch 2 ids 105, 403, 500..601 included), the State enum, the struct offsets, the state
//   transitions (Initialize None->Initialized, OpenAccessPoint 1->2, CreateNetwork 2->3, DestroyNetwork 3->2, CloseAccessPoint 2/3->1,
//   OpenStation 1->4, Connect 4->5, Disconnect 5->4, CloseStation 4/5->1, Finalize any->0), GetNetworkInfo needing state 3 or 5, Scan
//   needing 2..5, SetAdvertiseData <= 0x180 bytes in state 2..3, Reject in state 3.
//
// Mapping onto the lobby (lobby.h): SessionId = "pkpR" + u32 room id (so a NetworkInfo from Scan maps back to its room on Connect);
// BSSID = the host node's MAC (02:00:a9:fe:R:1); SSID = 32 hex digits from the local communication id and the room id; channel 1/6/11 by
// room id; link level 3; network type 2; the security parameter's random half = SHA-256 of the session id (host and guests agree). The
// passphrase reaches the server only as its truncated SHA-256 (vnet::passphrase_hash).
// Node table: the server's nicknames ([account] nickname of each player); the node's own entry uses the UserConfig name the game passed.
//
// When nobody leaves on purpose: the host closing the room or the server going away puts a station back to Station and a host back to
// AccessPoint with GetDisconnectReason set (DestroyedByUser 3 when the host destroyed the room, Rejected 5, DestroyedBySystem 4 on server
// shutdown, SignalLost 6 when the connection to the server is lost); the state change event is signalled on every state change and on
// every node join/leave.
// Host: the current link is a vnet::HoldLink in front of the lobby's, so datagrams from a node that just joined wait (up to kHoldMs) until
// the game has read a GetNetworkInfo / GetNetworkInfoLatestUpdate listing it; KTGL drops a sender outside its member table (design from
// pokopia-native (MIT) windows/src/coop/coop_mpnet.c, ACCEPT_WAIT_MS / cmp_host_accepts, @7c3655d).
// [net] room_code: the lobby's room key (vnet::room_key); a room hosted with one is hidden from players without it. A Connect refused for a
// wrong code answers ConnectNotFound.
// Session variant (services_api.h ldnapi): a game module's opt-in gameplay change that every player must share rides in bits 12..14 of the
// local communication version on the wire (CreateNetwork's room, Connect's JOIN), so the relay's own version check refuses a player whose
// variant differs. The game sees its own version only (NetworkInfo nodes, GetNetworkConfig). A Connect refused for another variant answers
// ConnectRejected and logs which side has what; the variant of a room comes from the last Scan that listed it.
// No lobby ([net] server empty): the state machine runs, Scan finds nothing, Connect answers ConnectNotFound and CreateNetwork
// DeviceNotAvailable (the answers of the offline stub this replaces).
#include "ldn.h"

#include <array>
#include <chrono>
#include <condition_variable>
#include <cstddef>
#include <cstdio>
#include <cstring>
#include <atomic>
#include <map>
#include <mutex>
#include <set>
#include <string>
#include <thread>
#include <vector>

#include "nxrt/config.h"
#include "nxrt/log.h"
#include "nxrt/services_api.h"
#include "../net/vnet.h"
#include "services_internal.h"

namespace nxrt::services {
namespace {

// ---- result codes (Ryujinx Ldn/ResultCode.cs; module 203) -----------------------------------------------------------------------------
constexpr uint32_t kDeviceNotAvailable = res(203, 16);
constexpr uint32_t kInvalidState = res(203, 32);
constexpr uint32_t kNodeNotFound = res(203, 48);
constexpr uint32_t kConnectFailure = res(203, 64);
constexpr uint32_t kConnectNotFound = res(203, 65);
constexpr uint32_t kConnectTimeout = res(203, 66);
constexpr uint32_t kConnectRejected = res(203, 67);
constexpr uint32_t kInvalidArgument = res(203, 96);
constexpr uint32_t kVersionTooLow = res(203, 113);
constexpr uint32_t kVersionTooHigh = res(203, 114);
constexpr uint32_t kTooManyPlayers = res(203, 144);

// ---- enums ----------------------------------------------------------------------------------------------------------------------------
enum State : uint32_t { kNone = 0, kInitialized = 1, kAccessPoint = 2, kAccessPointCreated = 3, kStation = 4, kStationConnected = 5, kError = 6 };
enum Reason : int16_t {
    kReasonNone = 0, kDisconnectedByUser = 1, kDisconnectedBySystem = 2, kDestroyedByUser = 3, kDestroyedBySystem = 4, kRejected = 5,
    kSignalLost = 6,
};
enum : uint8_t { kNodeConnect = 1, kNodeDisconnect = 2 };   // NodeLatestUpdate.StateChange
enum : uint32_t {                                            // ScanFilterFlag
    kFilterLocalCommunicationId = 1 << 0, kFilterSessionId = 1 << 1, kFilterNetworkType = 1 << 2, kFilterBssid = 1 << 3,
    kFilterSsid = 1 << 4, kFilterSceneId = 1 << 5,
};

// ---- layouts (Switchbrew "LDN services", Ryujinx Ldn/Types) ---------------------------------------------------------------------------
struct IntentId {
    uint64_t local_comm_id;
    uint16_t reserved1;
    uint16_t scene_id;
    uint32_t reserved2;
};
struct NetworkId {
    IntentId intent;
    uint8_t session_id[16];
};
struct Ssid {
    uint8_t length;
    char raw[33];
};
struct CommonNetworkInfo {
    uint8_t bssid[6];
    Ssid ssid;
    int16_t channel;
    int8_t link_level;
    uint8_t network_type;
    uint32_t reserved;
};
struct NodeInfo {
    uint32_t ipv4;
    uint8_t mac[6];
    int8_t node_id;
    uint8_t is_connected;
    char user_name[33];
    uint8_t reserved1;   // Platform from 19.0.0 (0 = NX)
    int16_t local_comm_version;
    uint8_t reserved2[16];
};
struct LdnNetworkInfo {
    uint8_t security_parameter[16];
    uint16_t security_mode;
    uint8_t station_accept_policy;
    uint8_t unknown1;
    uint8_t reserved1[2];
    uint8_t node_count_max;
    uint8_t node_count;
    NodeInfo nodes[8];
    uint8_t reserved2[2];
    uint16_t advertise_data_size;
    uint8_t advertise_data[384];
    uint8_t reserved3[140];
    uint64_t authentication_id;
};
struct NetworkInfo {
    NetworkId id;
    CommonNetworkInfo common;
    LdnNetworkInfo ldn;
};
struct NodeLatestUpdate {
    uint8_t state_change;
    uint8_t reserved[7];
};
struct ScanFilter {
    NetworkId id;
    uint32_t network_type;
    uint8_t bssid[6];
    Ssid ssid;
    uint8_t reserved[16];
    uint32_t flag;
};
struct NetworkConfig {
    IntentId intent;
    uint16_t channel;
    uint8_t node_count_max;
    uint8_t reserved1;
    uint16_t local_comm_version;
    uint8_t reserved2[10];
};
struct SecurityConfig {
    uint16_t mode;
    uint16_t passphrase_size;
    uint8_t passphrase[64];
};
struct SecurityParameter {
    uint8_t data[16];
    uint8_t session_id[16];
};
struct UserConfig {
    char user_name[33];
    uint8_t reserved[15];
};

static_assert(sizeof(IntentId) == 0x10 && sizeof(NetworkId) == 0x20 && sizeof(Ssid) == 0x22);
static_assert(sizeof(CommonNetworkInfo) == 0x30 && offsetof(CommonNetworkInfo, channel) == 0x28);
static_assert(sizeof(NodeInfo) == 0x40 && offsetof(NodeInfo, user_name) == 0x0C && offsetof(NodeInfo, local_comm_version) == 0x2E);
static_assert(sizeof(LdnNetworkInfo) == 0x430 && offsetof(LdnNetworkInfo, nodes) == 0x18 && offsetof(LdnNetworkInfo, advertise_data_size) == 0x21A &&
              offsetof(LdnNetworkInfo, advertise_data) == 0x21C && offsetof(LdnNetworkInfo, authentication_id) == 0x428);
static_assert(sizeof(NetworkInfo) == 0x480 && offsetof(NetworkInfo, ldn) == 0x50);
static_assert(sizeof(NodeLatestUpdate) == 8);
static_assert(sizeof(ScanFilter) == 0x60 && offsetof(ScanFilter, ssid) == 0x2A && offsetof(ScanFilter, flag) == 0x5C);
static_assert(sizeof(NetworkConfig) == 0x20 && offsetof(NetworkConfig, local_comm_version) == 0x14);
static_assert(sizeof(SecurityConfig) == 0x44 && sizeof(SecurityParameter) == 0x20 && sizeof(UserConfig) == 0x30);

// Raw argument offsets (CMIF, natural alignment: NetworkConfig is 8-aligned).
constexpr size_t kCreateUser = 0x44, kCreateNetCfg = 0x78;                                 // CreateNetwork
constexpr size_t kCreatePrivUser = 0x64, kCreatePrivNetCfg = 0x98;   // CreateNetworkPrivate (SecurityParameter at 0x44: unused)
constexpr size_t kConnectUser = 0x44, kConnectVersion = 0x74;                              // Connect
constexpr size_t kConnPrivSecParam = 0x44, kConnPrivUser = 0x64, kConnPrivVersion = 0x94, kConnPrivNetCfg = 0xA0;   // ConnectPrivate
constexpr uint32_t kSubnetMask = 0xFFFFFF00;   // the game asks MakeSubnetMask(24) too
constexpr uint8_t kSessionTag[4] = {'p', 'k', 'p', 'R'};
// Connect waits this long for the room state that lists the new node (the relay sends ROOM_STATE right behind JOINED); the lobby's own
// request timeout is 5 s too (vnet::RelayOptions::timeout_ms).
constexpr int kRoomStateWaitMs = 5000;
// Host: datagrams from a node the game has not yet seen in a NetworkInfo are held (vnet::HoldLink) until the game reads a table listing
// it, at most this long (pokopia-native windows/src/coop/coop_mpnet.c ACCEPT_WAIT_MS: the game polls GetNetworkInfo every frame), and at
// most kHoldCap of them. KTGL drops a datagram from a sender outside its member table; the new station's first one is its join request.
constexpr int kHoldMs = 300;
constexpr size_t kHoldCap = 32;

// ---- session variant (services_api.h ldnapi) ----------------------------------------------------------------------------------------
struct Variant {
    std::mutex mu;
    uint8_t value = 0;
    std::string what;
};
Variant& variant() {
    static Variant v;
    return v;
}
uint8_t own_variant(std::string* what = nullptr) {
    Variant& v = variant();
    std::lock_guard<std::mutex> lk(v.mu);
    if (what) *what = v.what;
    return v.value;
}
// The game's version on the wire: its variant in bits 12..14. Only for versions 0..0xfff (the game's own bits never collide); any other is
// sent as it is (logged once while a variant is set).
int16_t wire_version(int16_t game) {
    const uint8_t var = own_variant();
    if (!var) return game;
    if (game < 0 || (static_cast<uint16_t>(game) & ~0x0FFFu)) {
        static std::atomic<bool> warned{false};
        if (!warned.exchange(true))
            NXLOG_W("net", "ldn: the game's local communication version 0x%04x uses bits 12..15: the session variant is not sent",
                    static_cast<uint16_t>(game));
        return game;
    }
    return static_cast<int16_t>(game | (var << ldnapi::kVariantShift));
}
int16_t game_version(int16_t wire) { return wire < 0 ? wire : static_cast<int16_t>(wire & ~ldnapi::kVariantMask); }
uint8_t variant_of(int16_t wire) { return wire < 0 ? 0 : static_cast<uint8_t>((wire & ldnapi::kVariantMask) >> ldnapi::kVariantShift); }

// ---- process lobby ------------------------------------------------------------------------------------------------------------------
struct LobbySlot {
    std::mutex mu;
    bool built = false;
    std::shared_ptr<vnet::Lobby> lobby;
};
LobbySlot& slot() {
    static LobbySlot s;
    return s;
}

// room-code.txt beside the log (the game folder for pkp.exe): the code in use, for the launcher to show (launcher room_code_host).
std::string room_code_file() {
    const std::string log = log_path();
    const size_t cut = log.find_last_of("/\\");
    return (cut == std::string::npos ? std::string() : log.substr(0, cut + 1)) + "room-code.txt";
}

// [net] room_code -> the lobby's room key (zero: none). While a code is in use it is logged and written to room_code_file(), else
// that file is removed (a code from an earlier run must not be shown). False: the value is not a code (local wireless stays off).
bool room_code_setup(std::array<uint8_t, 16>* key) {
    std::string v = config_get_str("net", "room_code", "");
    const std::string file = room_code_file();
    std::string lower = v, norm;
    for (char& c : lower) c = (c >= 'A' && c <= 'Z') ? static_cast<char>(c - 'A' + 'a') : c;
    if (v.empty()) {
        std::remove(file.c_str());
        return true;
    }
    if (lower == "new") {
        norm = vnet::room_code_new();
        if (norm.empty()) {
            NXLOG_W("net", "ldn: [net] room_code = new: no random source: local wireless off");
            std::remove(file.c_str());
            return false;
        }
    } else if (!vnet::room_code_normalize(v, &norm)) {
        NXLOG_W("net", "ldn: [net] room_code is not a room code (16 letters and digits, XXXX-XXXX-XXXX-XXXX): local wireless off");
        std::remove(file.c_str());
        return false;
    }
    *key = vnet::room_key(norm);
    const std::string shown = vnet::room_code_format(norm);
    if (FILE* f = std::fopen(file.c_str(), "wb")) {
        std::fprintf(f, "%s\n", shown.c_str());
        std::fclose(f);
    }
    NXLOG_I("net", "ldn: room code %s%s: a room hosted here is seen only by players with this code; scans show open rooms and rooms with it",
            shown.c_str(), lower == "new" ? " (new)" : "");
    return true;
}

std::shared_ptr<vnet::Lobby> process_lobby() {
    LobbySlot& s = slot();
    std::lock_guard<std::mutex> lk(s.mu);
    if (s.built) return s.lobby;
    s.built = true;
    const std::string server = config_get_str("net", "server", "");
    vnet::RelayOptions o;
    if (server.empty()) {
        NXLOG_I("net", "ldn: [net] server is empty: local wireless off (scans find nothing)");
        std::remove(room_code_file().c_str());
        return nullptr;
    }
    if (!vnet::parse_server(server, &o.host, &o.port)) {
        NXLOG_W("net", "ldn: [net] server = \"%s\" is not host:port: local wireless off", server.c_str());
        return nullptr;
    }
    o.nickname = profile().user_nickname;
    if (o.nickname.empty()) o.nickname = "Player";
    o.password = config_get_str("net", "password", "");
    if (!room_code_setup(&o.room_key)) return nullptr;
    std::string udp = config_get_str("net", "udp", "auto");
    for (char& ch : udp) ch = static_cast<char>(ch >= 'A' && ch <= 'Z' ? ch - 'A' + 'a' : ch);
    o.udp = !(udp == "off" || udp == "0" || udp == "false" || udp == "no" || udp == "tcp");
    if (o.udp && udp != "auto" && udp != "on" && udp != "1" && udp != "true" && udp != "yes")
        NXLOG_W("net", "ldn: [net] udp = \"%s\" is not auto or off: auto", udp.c_str());
    s.lobby = vnet::make_relay_client(o);
    return s.lobby;
}

// ---- helpers ------------------------------------------------------------------------------------------------------------------------
template <class T>
T arg_at(const ipc::Request& req, size_t off) {
    return req.arg<T>(off);
}

void copy_name(char (&dst)[33], const std::string& s) {
    std::memset(dst, 0, sizeof dst);
    size_t n = s.size() < 32 ? s.size() : 32;
    while (n && n < s.size() && (static_cast<uint8_t>(s[n]) & 0xC0) == 0x80) --n;
    std::memcpy(dst, s.data(), n);
}

uint32_t room_of_session(const uint8_t (&sid)[16]) {
    if (std::memcmp(sid, kSessionTag, 4) != 0) return 0;
    uint32_t id;
    std::memcpy(&id, sid + 4, 4);
    return id;
}

// Whether a room state lists the host (node 0) and the node with address vip.
bool room_lists(const vnet::Room& r, uint32_t vip) {
    bool host = false, me = false;
    for (const vnet::RoomNode& n : r.nodes) {
        host |= n.node_id == 0;
        me |= n.vip == vip;
    }
    return host && me;
}

// The NetworkInfo of a lobby room. own_vip/own_name: this node's entry uses the game's UserConfig name.
NetworkInfo make_info(const vnet::Room& r, uint8_t accept_policy, uint32_t own_vip, const std::string& own_name) {
    NetworkInfo ni{};
    ni.id.intent.local_comm_id = r.local_comm_id;
    ni.id.intent.scene_id = r.scene_id;
    std::memcpy(ni.id.session_id, kSessionTag, 4);
    std::memcpy(ni.id.session_id + 4, &r.room_id, 4);
    const auto mac = std::array<uint8_t, 6>{0x02, 0x00, 0xA9, 0xFE, static_cast<uint8_t>(r.room_id), 1};
    std::memcpy(ni.common.bssid, mac.data(), 6);
    char ssid[33];
    std::snprintf(ssid, sizeof ssid, "%016llx%016llx", static_cast<unsigned long long>(r.local_comm_id),
                  static_cast<unsigned long long>(r.room_id));
    ni.common.ssid.length = 32;
    std::memcpy(ni.common.ssid.raw, ssid, 32);
    static constexpr int16_t kChannels[3] = {1, 6, 11};
    ni.common.channel = kChannels[r.room_id % 3];
    ni.common.link_level = 3;
    ni.common.network_type = 2;
    const auto sp = vnet::sha256(ni.id.session_id, sizeof ni.id.session_id);
    std::memcpy(ni.ldn.security_parameter, sp.data(), 16);
    ni.ldn.security_mode = r.security_mode;
    ni.ldn.station_accept_policy = accept_policy;
    ni.ldn.node_count_max = r.node_count_max;
    for (const vnet::RoomNode& n : r.nodes) {
        if (n.node_id >= 8) continue;
        NodeInfo& d = ni.ldn.nodes[n.node_id];
        d.ipv4 = n.vip;
        std::memcpy(d.mac, n.mac.data(), 6);
        d.node_id = static_cast<int8_t>(n.node_id);
        d.is_connected = 1;
        copy_name(d.user_name, n.vip == own_vip && !own_name.empty() ? own_name : n.nickname);
        d.local_comm_version = game_version(r.comm_version);
        ++ni.ldn.node_count;
    }
    const size_t adv = r.adv.size() < sizeof ni.ldn.advertise_data ? r.adv.size() : sizeof ni.ldn.advertise_data;
    ni.ldn.advertise_data_size = static_cast<uint16_t>(adv);
    if (adv) std::memcpy(ni.ldn.advertise_data, r.adv.data(), adv);
    ni.ldn.authentication_id = r.room_id;
    return ni;
}

bool filter_match(const ScanFilter& f, const NetworkInfo& ni) {
    if ((f.flag & kFilterSessionId) && std::memcmp(f.id.session_id, ni.id.session_id, 16) != 0) return false;
    if ((f.flag & kFilterNetworkType) && !(f.network_type & ni.common.network_type)) return false;
    if ((f.flag & kFilterBssid) && std::memcmp(f.bssid, ni.common.bssid, 6) != 0) return false;
    if ((f.flag & kFilterSsid) &&
        (f.ssid.length != ni.common.ssid.length || std::memcmp(f.ssid.raw, ni.common.ssid.raw, ni.common.ssid.length) != 0))
        return false;
    return true;   // local communication id and scene id were filtered by the server
}

// The first 128 bytes of the advertise data as hex for the Debug SetAdvertiseData line ("|" every 16 bytes, "..." when cut). Pokopia puts
// the host's KTGL member id in it (work/mp/join-reject-analysis.md section 3). Only built when Debug is on (NXLOG_D).
std::string hex_head(const std::vector<uint8_t>& b) {
    static const char kHex[] = "0123456789abcdef";
    const size_t n = b.size() < 128 ? b.size() : 128;
    std::string s;
    for (size_t i = 0; i < n; ++i) {
        if (i) s += (i % 16) ? " " : " | ";
        s += kHex[b[i] >> 4];
        s += kHex[b[i] & 15];
    }
    if (n < b.size()) s += " ...";
    return s;
}

// Logs the first use of a command (one line per command id for the life of the process; network_reset() clears it).
std::mutex g_seen_mu;
std::set<uint32_t> g_seen;
void note(uint32_t id, const char* name) {
    {
        std::lock_guard<std::mutex> lk(g_seen_mu);
        if (!g_seen.insert(id).second) return;
    }
    NXLOG_I("net", "ldn cmd %u %s (first use)", id, name);
}

// =============================================================================================================================================
// One IUserLocalCommunicationService. op_mu serialises the commands (it is held across lobby requests); mu guards the fields and is the only
// lock the lobby's event sinks take, so a sink never waits on a request in flight.
// =============================================================================================================================================

class Ldn : public std::enable_shared_from_this<Ldn> {
public:
    void add_commands(ipc::Service& s);
    void shutdown();   // the service object was released: leave and drop the link

private:
    using H = uint32_t (Ldn::*)(ipc::Request&, ipc::Response&);
    void cmd(ipc::Service& s, uint32_t id, const char* name, H h);

    uint32_t get_state(ipc::Request&, ipc::Response&);
    uint32_t get_network_info(ipc::Request&, ipc::Response&);
    uint32_t get_ipv4(ipc::Request&, ipc::Response&);
    uint32_t get_disconnect_reason(ipc::Request&, ipc::Response&);
    uint32_t get_security_parameter(ipc::Request&, ipc::Response&);
    uint32_t get_network_config(ipc::Request&, ipc::Response&);
    uint32_t attach_event(ipc::Request&, ipc::Response&);
    uint32_t get_latest_update(ipc::Request&, ipc::Response&);
    uint32_t scan(ipc::Request&, ipc::Response&);
    uint32_t accept(ipc::Request&, ipc::Response&) { return 0; }
    uint32_t open_access_point(ipc::Request&, ipc::Response&);
    uint32_t close_access_point(ipc::Request&, ipc::Response&);
    uint32_t create_network(ipc::Request&, ipc::Response&);
    uint32_t create_network_private(ipc::Request&, ipc::Response&);
    uint32_t destroy_network(ipc::Request&, ipc::Response&);
    uint32_t reject(ipc::Request&, ipc::Response&);
    uint32_t set_advertise_data(ipc::Request&, ipc::Response&);
    uint32_t set_accept_policy(ipc::Request&, ipc::Response&);
    uint32_t accept_filter(ipc::Request&, ipc::Response&);
    uint32_t open_station(ipc::Request&, ipc::Response&);
    uint32_t close_station(ipc::Request&, ipc::Response&);
    uint32_t connect(ipc::Request&, ipc::Response&);
    uint32_t connect_private(ipc::Request&, ipc::Response&);
    uint32_t disconnect(ipc::Request&, ipc::Response&);
    uint32_t initialize(ipc::Request&, ipc::Response&);
    uint32_t finalize(ipc::Request&, ipc::Response&);

    uint32_t create(const SecurityConfig& sec, const UserConfig& user, const NetworkConfig& cfg);
    uint32_t join(uint32_t room_id, int16_t room_version, const SecurityConfig& sec, const UserConfig& user, int32_t version);
    void on_room_state(const vnet::Room& r);
    void on_left(vnet::LeftReason why);
    void apply_room(const vnet::Room& r);       // caller holds mu
    void set_state(uint32_t s);                 // caller holds mu
    void admit_read();                          // caller holds mu: the game read info_, let its nodes' held datagrams through
    void drop_hold();                           // caller holds mu: the host's hold link is unhooked and gone
    void leave_network(uint32_t next, int16_t reason);   // caller holds op_mu (not mu)
    bool in_network() const { return state_ == kAccessPointCreated || state_ == kStationConnected; }

    std::mutex op_mu_;
    std::mutex mu_;
    std::condition_variable room_cv_;    // notified (under mu_) on every lobby event; join() waits on it for its first room state
    ServiceEvent ev_;
    uint32_t state_ = kNone;
    int16_t reason_ = kReasonNone;
    std::shared_ptr<vnet::Lobby> lobby_;
    std::vector<uint8_t> adv_;           // host: SetAdvertiseData before or after CreateNetwork
    uint8_t accept_policy_ = 0;
    std::string user_name_;              // the UserConfig name of the last create/connect
    uint32_t room_id_ = 0, vip_ = 0;
    NetworkInfo info_{};
    std::array<uint32_t, 8> node_vip_{};  // the node table last seen (0 = empty slot), for the join/leave flags
    std::array<uint8_t, 8> latest_{};    // NodeLatestUpdate flags since the last GetNetworkInfoLatestUpdate
    std::shared_ptr<vnet::HoldLink> hold_;   // host: the current link, in front of the lobby's (see kHoldMs)
    std::map<uint32_t, int16_t> scan_wire_;  // room id -> its version on the wire as the last Scan listed it (the session variant)
};

void Ldn::cmd(ipc::Service& s, uint32_t id, const char* name, H h) {
    std::weak_ptr<Ldn> w = weak_from_this();
    s.add_cmd(id, name, [w, id, name, h](ipc::Request& req, ipc::Response& rsp) -> uint32_t {
        note(id, name);
        auto self = w.lock();
        if (!self) return kInvalidState;
        std::lock_guard<std::mutex> op(self->op_mu_);
        return ((*self).*h)(req, rsp);
    });
}

void Ldn::add_commands(ipc::Service& s) {
    cmd(s, 0, "GetState", &Ldn::get_state);
    cmd(s, 1, "GetNetworkInfo", &Ldn::get_network_info);
    cmd(s, 2, "GetIpv4Address", &Ldn::get_ipv4);
    cmd(s, 3, "GetDisconnectReason", &Ldn::get_disconnect_reason);
    cmd(s, 4, "GetSecurityParameter", &Ldn::get_security_parameter);
    cmd(s, 5, "GetNetworkConfig", &Ldn::get_network_config);
    cmd(s, 100, "AttachStateChangeEvent", &Ldn::attach_event);
    cmd(s, 101, "GetNetworkInfoLatestUpdate", &Ldn::get_latest_update);
    cmd(s, 102, "Scan", &Ldn::scan);
    cmd(s, 103, "ScanPrivate", &Ldn::scan);
    cmd(s, 104, "SetWirelessControllerRestriction", &Ldn::accept);
    cmd(s, 105, "SetWirelessAudioPolicy", &Ldn::accept);
    cmd(s, 106, "SetProtocol", &Ldn::accept);
    cmd(s, 200, "OpenAccessPoint", &Ldn::open_access_point);
    cmd(s, 201, "CloseAccessPoint", &Ldn::close_access_point);
    cmd(s, 202, "CreateNetwork", &Ldn::create_network);
    cmd(s, 203, "CreateNetworkPrivate", &Ldn::create_network_private);
    cmd(s, 204, "DestroyNetwork", &Ldn::destroy_network);
    cmd(s, 205, "Reject", &Ldn::reject);
    cmd(s, 206, "SetAdvertiseData", &Ldn::set_advertise_data);
    cmd(s, 207, "SetStationAcceptPolicy", &Ldn::set_accept_policy);
    cmd(s, 208, "AddAcceptFilterEntry", &Ldn::accept_filter);
    cmd(s, 209, "ClearAcceptFilter", &Ldn::accept_filter);
    cmd(s, 300, "OpenStation", &Ldn::open_station);
    cmd(s, 301, "CloseStation", &Ldn::close_station);
    cmd(s, 302, "Connect", &Ldn::connect);
    cmd(s, 303, "ConnectPrivate", &Ldn::connect_private);
    cmd(s, 304, "Disconnect", &Ldn::disconnect);
    cmd(s, 400, "Initialize", &Ldn::initialize);
    cmd(s, 401, "Finalize", &Ldn::finalize);
    cmd(s, 402, "InitializeWithVersion", &Ldn::initialize);
    cmd(s, 403, "SetOperationMode", &Ldn::accept);
    // 500..601 (action frames, home channel, tx power; Switchbrew, 18.0.0+) stay unregistered: the IPC layer logs them if a game asks.
}

void Ldn::admit_read() {
    if (!hold_) return;
    std::vector<uint32_t> v;
    for (const uint32_t ip : node_vip_)
        if (ip) v.push_back(ip);
    hold_->admit(v);
}

void Ldn::drop_hold() {
    if (!hold_) return;
    hold_->detach();   // before the lobby's link can be the current link again (a Connect as station)
    hold_.reset();
}

void Ldn::set_state(uint32_t s) {
    if (state_ != s) NXLOG_I("net", "ldn: state %u -> %u", state_, s);
    state_ = s;
    ev_.signal();
}

// ---- lobby events (lobby thread) ------------------------------------------------------------------------------------------------------

void Ldn::on_room_state(const vnet::Room& r) {
    std::lock_guard<std::mutex> lk(mu_);
    room_cv_.notify_all();
    if (!in_network() || r.room_id != room_id_) return;
    apply_room(r);
    ev_.signal();
}

void Ldn::on_left(vnet::LeftReason why) {
    std::lock_guard<std::mutex> lk(mu_);
    room_cv_.notify_all();
    if (!in_network()) return;
    int16_t reason = kSignalLost;
    switch (why) {
        case vnet::LeftReason::HostClosed: reason = kDestroyedByUser; break;
        case vnet::LeftReason::Rejected: reason = kRejected; break;
        case vnet::LeftReason::Shutdown: reason = kDestroyedBySystem; break;
        default: reason = state_ == kAccessPointCreated ? kDestroyedBySystem : kSignalLost; break;
    }
    NXLOG_I("net", "ldn: left room %u (reason %d)", room_id_, reason);
    vnet::set_current_link(nullptr);
    drop_hold();
    reason_ = reason;
    room_id_ = vip_ = 0;
    info_ = NetworkInfo{};
    node_vip_ = {};
    set_state(state_ == kAccessPointCreated ? kAccessPoint : kStation);
}

void Ldn::apply_room(const vnet::Room& r) {
    info_ = make_info(r, accept_policy_, vip_, user_name_);
    std::array<uint32_t, 8> now{};
    for (const vnet::RoomNode& n : r.nodes)
        if (n.node_id < 8) now[n.node_id] = n.vip;
    for (size_t i = 0; i < 8; ++i) {
        if (now[i] == node_vip_[i]) continue;
        if (node_vip_[i]) latest_[i] |= kNodeDisconnect;
        if (now[i]) latest_[i] |= kNodeConnect;
    }
    node_vip_ = now;
    if (hold_) {
        std::vector<uint32_t> v;
        for (const uint32_t ip : now)
            if (ip) v.push_back(ip);
        hold_->members(v);
    }
}

// ---- state and info ---------------------------------------------------------------------------------------------------------------------

uint32_t Ldn::get_state(ipc::Request&, ipc::Response& rsp) {
    std::lock_guard<std::mutex> lk(mu_);
    rsp.push(state_);
    return 0;
}

uint32_t Ldn::get_network_info(ipc::Request& req, ipc::Response&) {
    std::lock_guard<std::mutex> lk(mu_);
    if (!in_network()) return kInvalidState;
    buf_write(req.out_buf(0), &info_, sizeof info_);
    admit_read();
    return 0;
}

uint32_t Ldn::get_ipv4(ipc::Request&, ipc::Response& rsp) {
    std::lock_guard<std::mutex> lk(mu_);
    if (!in_network()) return kInvalidState;
    rsp.push(vip_);
    rsp.push(kSubnetMask);
    return 0;
}

uint32_t Ldn::get_disconnect_reason(ipc::Request&, ipc::Response& rsp) {
    std::lock_guard<std::mutex> lk(mu_);
    rsp.push(reason_);
    return 0;
}

uint32_t Ldn::get_security_parameter(ipc::Request&, ipc::Response& rsp) {
    std::lock_guard<std::mutex> lk(mu_);
    if (!in_network()) return kInvalidState;
    SecurityParameter sp{};
    std::memcpy(sp.data, info_.ldn.security_parameter, 16);
    std::memcpy(sp.session_id, info_.id.session_id, 16);
    rsp.push(sp);
    return 0;
}

uint32_t Ldn::get_network_config(ipc::Request&, ipc::Response& rsp) {
    std::lock_guard<std::mutex> lk(mu_);
    if (!in_network()) return kInvalidState;
    NetworkConfig c{};
    c.intent = info_.id.intent;
    c.channel = static_cast<uint16_t>(info_.common.channel);
    c.node_count_max = info_.ldn.node_count_max;
    c.local_comm_version = static_cast<uint16_t>(info_.ldn.nodes[0].local_comm_version);
    rsp.push(c);
    return 0;
}

uint32_t Ldn::attach_event(ipc::Request&, ipc::Response& rsp) {
    rsp.out_copy.push_back(ev_.new_handle());
    return 0;
}

uint32_t Ldn::get_latest_update(ipc::Request& req, ipc::Response&) {
    std::lock_guard<std::mutex> lk(mu_);
    if (!in_network()) return kInvalidState;
    buf_write(req.out_buf(0), &info_, sizeof info_);
    NodeLatestUpdate up[8] = {};
    for (size_t i = 0; i < 8; ++i) up[i].state_change = latest_[i];
    buf_write(req.out_buf(1), up, sizeof up);
    latest_ = {};
    admit_read();
    return 0;
}

// ---- scan ----------------------------------------------------------------------------------------------------------------------------

uint32_t Ldn::scan(ipc::Request& req, ipc::Response& rsp) {
    std::shared_ptr<vnet::Lobby> lobby;
    uint32_t own_room;
    {
        std::lock_guard<std::mutex> lk(mu_);
        if (state_ == kNone) return kInvalidState;
        lobby = lobby_;
        // With no lobby every initialized state answers an empty scan (the offline stub's answer); with one, Switchbrew's states 2..5.
        if (lobby && (state_ < kAccessPoint || state_ > kStationConnected)) return kInvalidState;
        own_room = room_id_;
    }
    const ScanFilter f = arg_at<ScanFilter>(req, 8);   // after the s16 channel, 8-aligned
    const ipc::Buffer out = req.out_buf(0);
    const size_t cap = static_cast<size_t>(out.size / sizeof(NetworkInfo));
    uint16_t count = 0;
    if (lobby && cap) {
        std::vector<vnet::Room> rooms;
        const uint64_t lcid = (f.flag & kFilterLocalCommunicationId) ? f.id.intent.local_comm_id : 0;
        const uint16_t scene = (f.flag & kFilterSceneId) ? f.id.intent.scene_id : 0xFFFF;
        const int e = lobby->scan(lcid, scene, &rooms);
        if (e) NXLOG_D("net", "ldn: scan failed: %s", vnet::lobby_err_str(e));
        {
            std::lock_guard<std::mutex> lk(mu_);
            for (const vnet::Room& r : rooms) scan_wire_[r.room_id] = r.comm_version;
        }
        for (const vnet::Room& r : rooms) {
            if (count >= cap) break;
            if (r.room_id == own_room) continue;
            const NetworkInfo ni = make_info(r, 0, 0, {});
            if (!filter_match(f, ni)) continue;
            std::memcpy(buf_mut(out) + count * sizeof(NetworkInfo), &ni, sizeof ni);
            ++count;
        }
    }
    rsp.push(static_cast<uint32_t>(count));   // s16 total (the high half is zero)
    return 0;
}

// ---- access point ----------------------------------------------------------------------------------------------------------------------

uint32_t Ldn::open_access_point(ipc::Request&, ipc::Response&) {
    std::lock_guard<std::mutex> lk(mu_);
    if (state_ != kInitialized) return kInvalidState;
    set_state(kAccessPoint);
    return 0;
}

void Ldn::leave_network(uint32_t next, int16_t reason) {
    std::shared_ptr<vnet::Lobby> lobby;
    {
        std::lock_guard<std::mutex> lk(mu_);
        if (in_network()) {
            vnet::set_current_link(nullptr);
            drop_hold();
            lobby = lobby_;
            room_id_ = vip_ = 0;
            info_ = NetworkInfo{};
            node_vip_ = {};
            reason_ = reason;
        }
        set_state(next);
    }
    if (lobby) lobby->leave();   // outside mu: the loopback relay may run sinks of this very object
}

uint32_t Ldn::close_access_point(ipc::Request&, ipc::Response&) {
    {
        std::lock_guard<std::mutex> lk(mu_);
        if (state_ != kAccessPoint && state_ != kAccessPointCreated) return kInvalidState;
    }
    leave_network(kInitialized, kDestroyedByUser);
    return 0;
}

uint32_t Ldn::create_network(ipc::Request& req, ipc::Response&) {
    return create(arg_at<SecurityConfig>(req, 0), arg_at<UserConfig>(req, kCreateUser), arg_at<NetworkConfig>(req, kCreateNetCfg));
}

uint32_t Ldn::create_network_private(ipc::Request& req, ipc::Response&) {
    return create(arg_at<SecurityConfig>(req, 0), arg_at<UserConfig>(req, kCreatePrivUser), arg_at<NetworkConfig>(req, kCreatePrivNetCfg));
}

uint32_t Ldn::create(const SecurityConfig& sec, const UserConfig& user, const NetworkConfig& cfg) {
    std::shared_ptr<vnet::Lobby> lobby;
    vnet::RoomConfig rc;
    {
        std::lock_guard<std::mutex> lk(mu_);
        if (state_ != kAccessPoint) return kInvalidState;
        if (cfg.node_count_max < 1 || cfg.node_count_max > vnet::kMaxNodes || sec.passphrase_size > sizeof sec.passphrase)
            return kInvalidArgument;
        if (!lobby_) return kDeviceNotAvailable;
        lobby = lobby_;
        rc.local_comm_id = cfg.intent.local_comm_id;
        rc.scene_id = cfg.intent.scene_id;
        rc.comm_version = wire_version(static_cast<int16_t>(cfg.local_comm_version));
        rc.node_count_max = cfg.node_count_max;
        rc.security_mode = static_cast<uint8_t>(sec.mode);
        rc.pass_hash = vnet::passphrase_hash(sec.passphrase, sec.passphrase_size);
        rc.adv = adv_;
        user_name_.assign(user.user_name, strnlen(user.user_name, 32));
    }
    vnet::Joined j;
    const int e = lobby->create(rc, &j);
    if (e) {
        NXLOG_I("net", "ldn: CreateNetwork failed: %s", vnet::lobby_err_str(e));
        return kDeviceNotAvailable;
    }
    std::lock_guard<std::mutex> lk(mu_);
    room_id_ = j.room_id;
    vip_ = j.vip;
    reason_ = kReasonNone;
    node_vip_ = {};
    latest_ = {};
    vnet::Room r = lobby->room();
    // No ROOM_STATE yet: the host alone. This is the whole room (node 0 = this node, connected, with the advertise data), so CreateNetwork
    // needs no wait; a ROOM_STATE that comes later replaces it.
    if (r.room_id != j.room_id || r.nodes.empty()) {
        r.room_id = j.room_id;
        r.local_comm_id = rc.local_comm_id;
        r.scene_id = rc.scene_id;
        r.comm_version = rc.comm_version;
        r.node_count_max = rc.node_count_max;
        r.security_mode = rc.security_mode;
        r.adv = rc.adv;
        r.nodes = {{0, j.vip, {0x02, 0x00, 0xA9, 0xFE, static_cast<uint8_t>(j.vip >> 8), static_cast<uint8_t>(j.vip)}, user_name_}};
    }
    drop_hold();
    hold_ = std::make_shared<vnet::HoldLink>(lobby->link(), kHoldMs, kHoldCap);
    apply_room(r);
    vnet::set_current_link(hold_);
    std::string what;
    if (const uint8_t var = variant_of(rc.comm_version); var && own_variant(&what) == var)
        NXLOG_I("net", "ldn: created room %u as %s with %s (session variant %u: only players with it can join)", room_id_,
                vnet::ip_str(vip_).c_str(), what.c_str(), var);
    else NXLOG_I("net", "ldn: created room %u as %s", room_id_, vnet::ip_str(vip_).c_str());
    set_state(kAccessPointCreated);
    return 0;
}

uint32_t Ldn::destroy_network(ipc::Request&, ipc::Response&) {
    {
        std::lock_guard<std::mutex> lk(mu_);
        if (state_ != kAccessPointCreated) return kInvalidState;
    }
    leave_network(kAccessPoint, kDestroyedByUser);
    return 0;
}

uint32_t Ldn::reject(ipc::Request& req, ipc::Response&) {
    const uint32_t ip = req.arg<uint32_t>(0);
    std::shared_ptr<vnet::Lobby> lobby;
    {
        std::lock_guard<std::mutex> lk(mu_);
        if (state_ != kAccessPointCreated) return kInvalidState;
        bool found = false;
        for (size_t i = 1; i < 8; ++i) found |= node_vip_[i] == ip && ip != 0;
        if (!found) return kNodeNotFound;
        lobby = lobby_;
    }
    lobby->reject(ip);   // the server answers with a ROOM_STATE without that node
    return 0;
}

uint32_t Ldn::set_advertise_data(ipc::Request& req, ipc::Response&) {
    const ipc::Buffer b = req.in_buf(0);
    if (b.size > vnet::kMaxAdvertise) return kInvalidArgument;
    std::shared_ptr<vnet::Lobby> lobby;
    std::vector<uint8_t> adv(buf_ptr(b), buf_ptr(b) + (b.addr ? b.size : 0));
    NXLOG_D("net", "ldn: SetAdvertiseData %zu bytes: %s", adv.size(), hex_head(adv).c_str());
    {
        std::lock_guard<std::mutex> lk(mu_);
        if (state_ != kAccessPoint && state_ != kAccessPointCreated) return kInvalidState;
        adv_ = adv;
        if (state_ == kAccessPointCreated) {
            lobby = lobby_;
            info_.ldn.advertise_data_size = static_cast<uint16_t>(adv.size());
            std::memset(info_.ldn.advertise_data, 0, sizeof info_.ldn.advertise_data);
            if (!adv.empty()) std::memcpy(info_.ldn.advertise_data, adv.data(), adv.size());
        }
    }
    if (lobby) lobby->set_advertise(adv);
    return 0;
}

uint32_t Ldn::set_accept_policy(ipc::Request& req, ipc::Response&) {
    std::lock_guard<std::mutex> lk(mu_);
    if (state_ != kAccessPoint && state_ != kAccessPointCreated) return kInvalidState;
    accept_policy_ = req.arg<uint8_t>(0);   // stored and reported; the relay has no accept policy yet (everyone may join)
    info_.ldn.station_accept_policy = accept_policy_;
    return 0;
}

uint32_t Ldn::accept_filter(ipc::Request&, ipc::Response&) {
    std::lock_guard<std::mutex> lk(mu_);
    return state_ == kAccessPoint || state_ == kAccessPointCreated ? 0 : kInvalidState;
}

// ---- station ----------------------------------------------------------------------------------------------------------------------------

uint32_t Ldn::open_station(ipc::Request&, ipc::Response&) {
    std::lock_guard<std::mutex> lk(mu_);
    if (state_ != kInitialized) return kInvalidState;
    set_state(kStation);
    return 0;
}

uint32_t Ldn::close_station(ipc::Request&, ipc::Response&) {
    {
        std::lock_guard<std::mutex> lk(mu_);
        if (state_ != kStation && state_ != kStationConnected) return kInvalidState;
    }
    leave_network(kInitialized, kDisconnectedByUser);
    return 0;
}

uint32_t Ldn::connect(ipc::Request& req, ipc::Response&) {
    NetworkInfo ni{};
    buf_read(req.in_buf(0), &ni, sizeof ni);
    return join(room_of_session(ni.id.session_id), ni.ldn.nodes[0].local_comm_version, arg_at<SecurityConfig>(req, 0),
                arg_at<UserConfig>(req, kConnectUser), req.arg<int32_t>(kConnectVersion));
}

uint32_t Ldn::connect_private(ipc::Request& req, ipc::Response&) {
    const SecurityParameter sp = arg_at<SecurityParameter>(req, kConnPrivSecParam);
    const NetworkConfig cfg = arg_at<NetworkConfig>(req, kConnPrivNetCfg);
    return join(room_of_session(sp.session_id), static_cast<int16_t>(cfg.local_comm_version), arg_at<SecurityConfig>(req, 0),
                arg_at<UserConfig>(req, kConnPrivUser), req.arg<int32_t>(kConnPrivVersion));
}

uint32_t Ldn::join(uint32_t room_id, int16_t room_version, const SecurityConfig& sec, const UserConfig& user, int32_t version) {
    std::shared_ptr<vnet::Lobby> lobby;
    {
        std::lock_guard<std::mutex> lk(mu_);
        if (state_ == kNone) return kInvalidState;
        if (!lobby_) return kConnectNotFound;   // no radio: nothing to connect to (the offline stub's answer, any state)
        if (state_ != kStation) return kInvalidState;
        if (sec.passphrase_size > sizeof sec.passphrase) return kInvalidArgument;
        lobby = lobby_;
        user_name_.assign(user.user_name, strnlen(user.user_name, 32));
    }
    if (room_id == 0) return kConnectNotFound;
    vnet::Joined j;
    const int16_t wire = wire_version(static_cast<int16_t>(version));
    const int e = lobby->join(room_id, wire, vnet::passphrase_hash(sec.passphrase, sec.passphrase_size), &j);
    if (e) {
        NXLOG_I("net", "ldn: Connect to room %u failed: %s", room_id, vnet::lobby_err_str(e));
        if (e == vnet::kLobbyVersion) {
            int16_t room_wire = room_version;
            bool known = false;
            {
                std::lock_guard<std::mutex> lk(mu_);
                const auto it = scan_wire_.find(room_id);
                if (it != scan_wire_.end()) room_wire = it->second, known = true;
            }
            std::string what;
            const uint8_t mine = own_variant(&what), theirs = variant_of(room_wire);
            if (known && mine != theirs && game_version(room_wire) == game_version(wire)) {
                // the same game, another session variant: every player needs the same setting
                const char* name = what.empty() ? "a gameplay setting" : what.c_str();
                if (mine && theirs) NXLOG_W("net", "ldn: Connect to room %u refused: its session variant is %u, this PC's %u (%s)", room_id, theirs, mine, name);
                else if (mine) NXLOG_W("net", "ldn: Connect to room %u refused: the host has %s off, this PC has it on. Every player of a session needs the same setting", room_id, name);
                else NXLOG_W("net", "ldn: Connect to room %u refused: the host has %s on (session variant %u), this PC has it off. Every player of a session needs the same setting", room_id, name, theirs);
                return kConnectRejected;
            }
        }
        switch (e) {
            case vnet::kLobbyNoRoom:
            case vnet::kLobbyRoomCode: return kConnectNotFound;   // a room with another code: as if it were not there
            case vnet::kLobbyRoomFull: return kTooManyPlayers;
            case vnet::kLobbyVersion: return version < room_version ? kVersionTooLow : kVersionTooHigh;
            case vnet::kLobbyRejected: return kConnectRejected;
            case vnet::kLobbyTimeout: return kConnectTimeout;
            default: return kConnectFailure;   // wrong passphrase, no server
        }
    }
    // JOINED carries only the room id and this node's address; the node table and the advertise data come in the ROOM_STATE behind it.
    // The game reads both as soon as Connect returns (GetNetworkInfo: node[0] is the host it sends to, isConnected and the advertise data
    // with the host's session member id), so Connect returns only once the room state that lists the host and this node has been applied.
    // The room events notify room_cv_; the 50 ms slices also catch a state that came without an event. No such state within
    // kRoomStateWaitMs, or the lobby left the room meanwhile (its room() is then cleared): leave and fail like a failed join.
    std::unique_lock<std::mutex> lk(mu_);
    vnet::Room r;
    const auto deadline = std::chrono::steady_clock::now() + std::chrono::milliseconds(kRoomStateWaitMs);
    for (;;) {
        r = lobby->room();
        if (r.room_id != j.room_id || room_lists(r, j.vip) || std::chrono::steady_clock::now() >= deadline) break;
        room_cv_.wait_for(lk, std::chrono::milliseconds(50));
    }
    if (r.room_id != j.room_id || !room_lists(r, j.vip)) {
        lk.unlock();
        NXLOG_I("net", "ldn: Connect to room %u: %s, leaving", j.room_id,
                r.room_id != j.room_id ? "out of the room before its state came" : "no room state listing this node within 5 s");
        lobby->leave();
        return kConnectFailure;
    }
    room_id_ = j.room_id;
    vip_ = j.vip;
    reason_ = kReasonNone;
    node_vip_ = {};
    latest_ = {};
    apply_room(r);
    vnet::set_current_link(lobby->link());
    NXLOG_I("net", "ldn: joined room %u as %s (node %u)", room_id_, vnet::ip_str(vip_).c_str(), j.node_id);
    set_state(kStationConnected);
    return 0;
}

uint32_t Ldn::disconnect(ipc::Request&, ipc::Response&) {
    {
        std::lock_guard<std::mutex> lk(mu_);
        if (state_ != kStationConnected) return kInvalidState;
    }
    leave_network(kStation, kDisconnectedByUser);
    return 0;
}

// ---- lifetime ---------------------------------------------------------------------------------------------------------------------------

uint32_t Ldn::initialize(ipc::Request&, ipc::Response&) {
    std::shared_ptr<vnet::Lobby> lobby;
    {
        std::lock_guard<std::mutex> lk(mu_);
        if (state_ != kNone) return 0;   // already initialized: accepted
        lobby_ = process_lobby();
        lobby = lobby_;
        reason_ = kReasonNone;
        set_state(kInitialized);
    }
    if (!lobby) return 0;
    std::weak_ptr<Ldn> w = weak_from_this();
    vnet::LobbyEvents ev;
    ev.room_state = [w](const vnet::Room& r) {
        if (auto s = w.lock()) s->on_room_state(r);
    };
    ev.left = [w](vnet::LeftReason why) {
        if (auto s = w.lock()) s->on_left(why);
    };
    lobby->set_events(std::move(ev));
    // Connect now, off the game's thread (Initialize runs at start-up): the first Scan/Create then finds the connection up, or waits for it.
    if (!lobby->connected()) std::thread([lobby] { lobby->connect(); }).detach();
    return 0;
}

uint32_t Ldn::finalize(ipc::Request&, ipc::Response&) {
    {
        std::lock_guard<std::mutex> lk(mu_);
        if (state_ == kNone) return kInvalidState;
    }
    shutdown();
    return 0;
}

void Ldn::shutdown() {
    leave_network(kNone, kDisconnectedByUser);
    std::shared_ptr<vnet::Lobby> lobby;
    {
        std::lock_guard<std::mutex> lk(mu_);
        lobby.swap(lobby_);
    }
    if (lobby) lobby->set_events({});
}

class LdnService final : public ipc::Service {
public:
    LdnService() : ipc::Service("nn::ldn::detail::IUserLocalCommunicationService"), ldn_(std::make_shared<Ldn>()) {
        ldn_->add_commands(*this);
    }
    void on_close() override {
        ldn_->shutdown();
    }

private:
    std::shared_ptr<Ldn> ldn_;
};

}  // namespace

std::shared_ptr<ipc::Service> make_ldn_user_creator() {
    return make_iface("nn::ldn::detail::IUserServiceCreator", [](ipc::Service& s) {
        s.add_cmd(0, "CreateUserLocalCommunicationService", [](ipc::Request&, ipc::Response& rsp) -> uint32_t {
            note(1000, "IUserServiceCreator CreateUserLocalCommunicationService");
            rsp.push_object(std::make_shared<LdnService>());
            return 0;
        });
        s.add_cmd(1, "CreateClientProcessMonitor", [](ipc::Request&, ipc::Response& rsp) -> uint32_t {
            note(1001, "IUserServiceCreator CreateClientProcessMonitor");
            rsp.push_object(make_iface("nn::ldn::detail::IClientProcessMonitor", [](ipc::Service& m) {
                m.add_cmd(0, "RegisterClient", [](ipc::Request&, ipc::Response&) -> uint32_t { return 0; });
            }));
            return 0;
        });
    });
}

void ldn_set_lobby(std::shared_ptr<vnet::Lobby> lobby) {
    LobbySlot& s = slot();
    std::lock_guard<std::mutex> lk(s.mu);
    s.built = true;
    s.lobby = std::move(lobby);
}

void ldn_reset() {
    {
        std::lock_guard<std::mutex> lk(g_seen_mu);
        g_seen.clear();
    }
    std::shared_ptr<vnet::Lobby> old;
    {
        LobbySlot& s = slot();
        std::lock_guard<std::mutex> lk(s.mu);
        old.swap(s.lobby);
        s.built = false;
    }
    ldnapi::set_session_variant(0, "");
    // old goes away here (a relay client closes its connection)
}

namespace ldnapi {
void set_session_variant(uint8_t v, const char* what) {
    v &= kVariantMask >> kVariantShift;
    Variant& s = variant();
    std::lock_guard<std::mutex> lk(s.mu);
    s.value = v;
    s.what = what ? what : "";
}
uint8_t session_variant() { return own_variant(); }
}  // namespace ldnapi

}  // namespace nxrt::services
