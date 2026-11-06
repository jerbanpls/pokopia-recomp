// SPDX-License-Identifier: GPL-3.0-or-later
// HIPC message encoding/decoding (the wire format of the Horizon IPC command buffer, thread-local region +0).
// Layouts: https://switchbrew.org/wiki/IPC_Marshalling (HeaderData, SpecialHeader, MapData = A/B/W buffers, PointerData = X,
// ReceiveListData, raw data alignment) cross-checked against libnx sf/hipc.h.
//
// Message word order: [Header0 Header1] [Special header, pid (2 words), copy handles, move handles] [X descriptors, 2 words each]
// [A descriptors, 3 words each] [B] [W] [raw data] [receive list descriptors, 2 words each]
//   Header0: bits 0-15 type, 16-19 X count, 20-23 A count, 24-27 B count, 28-31 W count
//   Header1: bits 0-9 raw size in words, 10-13 receive list flags, 20-30 receive list offset (unused by us), 31 special header present
//   Special: bit 0 pid, 1-4 copy count, 5-8 move count
//   X (pointer) desc: w0 = index[0-3] | addr bits 36-38 << 6 | addr bits 32-35 << 12 | size << 16; w1 = addr low 32
//   A/B/W (map) desc: w0 = size low 32; w1 = addr low 32; w2 = attr[0-1] | addr bits 36-38 << 2 | size bits 32-35 << 24 | addr bits 32-35 << 28
//   Receive list desc: w0 = addr low 32; w1 = addr bits 32-47 | size << 16
// Raw data: CMIF messages (types 0 response, 1, 3, 4, 5, 6, 7 requests) start the raw section at the next 16-byte boundary measured
// from the message start; the raw section is then [pad][payload][pad][u16 out-pointer sizes] where the two pads always sum to 0x10 bytes.
// TIPC messages (type >= 16) and the close message (type 2) have no alignment padding: raw data follows the descriptors directly.
#pragma once
#include <cstddef>
#include <cstdint>
#include <string>
#include <vector>

namespace nxrt::hipc {

enum MessageType : uint16_t {
    kInvalid = 0, kInvokeMethod = 1, kRelease = 2, kInvokeManagerMethod = 3, kInvoke2MethodOld = 4, kInvoke2ManagerMethodOld = 5,
    kInvoke2Method = 6, kInvoke2ManagerMethod = 7, kTipcBase = 16,
};

struct PtrDesc { uint64_t addr = 0; uint32_t size = 0; uint32_t index = 0; };     // X
struct BufDesc { uint64_t addr = 0; uint64_t size = 0; uint32_t attr = 0; };      // A / B / W
struct RecvDesc { uint64_t addr = 0; uint32_t size = 0; };                         // receive list entry

struct Message {
    uint16_t type = 0;
    bool has_pid = false;
    uint64_t pid = 0;
    std::vector<uint32_t> copy, move;           // handles
    std::vector<PtrDesc> ptrs;                  // X
    std::vector<BufDesc> send, recv, exch;      // A, B, W
    uint32_t recv_list_flags = 0;               // 0 none, 1 inline, 2 one buffer, n >= 3: n-2 buffers
    std::vector<RecvDesc> recv_list;            // entries present after the raw data (flags >= 2)
    // Raw section. After hipc_parse: `raw` is everything after the leading alignment pad up to the end of the raw words
    // (so it includes the trailing pad and the out-pointer size array), `pad_before` the leading pad size in bytes.
    // For hipc_build: `raw` is the payload (CMIF header + data) and `out_ptr_sizes` the u16 array; the pads are generated.
    std::vector<uint8_t> raw;
    std::vector<uint16_t> out_ptr_sizes;
    uint32_t pad_before = 0;
};

// Number of header words and flags helpers.
inline bool is_tipc(uint16_t type) { return type >= kTipcBase; }
inline bool has_aligned_raw(uint16_t type) { return type != kRelease && type < kTipcBase; }

// Parses a message of `nwords` words starting at `words` (the buffer start is assumed 16-byte aligned for the raw alignment rule).
// Returns false with a reason in *err on any malformed / truncated input; never reads outside [words, words + nwords).
bool parse(const uint32_t* words, size_t nwords, Message& out, std::string* err = nullptr);
// Serialises a message; returns the number of words written, or 0 when it does not fit in `cap_words` or has invalid fields
// (counts over the 4-bit limits, handle counts over 15, raw payload over the 10-bit size limit).
size_t build(const Message& m, uint32_t* out, size_t cap_words, std::string* err = nullptr);
// Size in words build() would produce, or 0 for an invalid message.
size_t built_size_words(const Message& m);

}  // namespace nxrt::hipc
