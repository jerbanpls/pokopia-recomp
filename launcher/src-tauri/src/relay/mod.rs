// SPDX-License-Identifier: GPL-3.0-or-later
//! LDN relay server ("pkp host"). Game clients emulate Nintendo local wireless (nn::ldn) and tunnel the game's UDP
//! through this server. Every player, the host player too, keeps one TCP connection to it. The server keeps rooms
//! (LDN networks) and routes datagrams between virtual IPs inside a room; it never parses game data. A v3 client in a
//! room can send and get the game datagrams over UDP on the same port instead (see "UDP datagram path"); TCP stays the
//! control channel and the fallback.
//!
//! # Wire protocol v3 (reference for the C++ client)
//!
//! TCP, all integers little-endian.
//!
//! Frame = `u32 len` + `u8 type` + payload. `len` counts the bytes after the length field (type + payload), 1..=8192,
//! in both directions. A frame with len 0, len > 8192, an unknown type, a short payload, trailing bytes or a field
//! over its cap is a bad frame: the server answers ERROR 10 and closes the connection.
//! `str` = `u8 len` + UTF-8 bytes (invalid UTF-8 is a bad frame).
//!
//! Client -> server:
//!
//! | type | name      | payload |
//! |------|-----------|---------|
//! | 0x01 | HELLO     | u16 proto (2 or 3), str build stamp, str nickname (<= 32 B), str password |
//! | 0x02 | SCAN      | u64 local_comm_id (0 = any), u16 scene_id (0xffff = any), 16 B room key |
//! | 0x03 | CREATE    | u64 local_comm_id, u16 scene_id, s16 comm_version, u8 node_count_max (1..8), u8 security_mode, 16 B passphrase hash, 16 B room key, u16 adv_len + adv bytes (<= 384) |
//! | 0x04 | JOIN      | u32 room_id, s16 comm_version, 16 B passphrase hash, 16 B room key |
//! | 0x05 | LEAVE     | - |
//! | 0x06 | ADVERTISE | u16 len + bytes (<= 384), host only |
//! | 0x07 | REJECT    | u32 vip, host only |
//! | 0x08 | DATAGRAM  | u16 src_port, u32 dst_ip, u16 dst_port, u16 len + bytes (<= 4096) |
//! | 0x09 | PING      | u32 nonce |
//! | 0x0a | UDP_MODE  | u8 on (0/1), v3 only |
//!
//! Server -> client:
//!
//! | type | name       | payload |
//! |------|------------|---------|
//! | 0x81 | WELCOME    | u32 client_id, str server version |
//! | 0x82 | ERROR      | u16 code, str message |
//! | 0x83 | ROOM_LIST  | u8 count, count x ROOM_INFO |
//! | 0x84 | JOINED     | u32 room_id, u32 your_vip, u8 your_node_id |
//! | 0x85 | ROOM_STATE | ROOM_INFO |
//! | 0x86 | DATAGRAM   | u32 src_ip, u16 src_port, u32 dst_ip, u16 dst_port, u16 len + bytes |
//! | 0x87 | LEFT       | u16 reason (0 you left, 1 host closed, 2 rejected, 3 server shutdown) |
//! | 0x88 | PONG       | u32 nonce |
//! | 0x89 | UDP_OFFER  | u64 token, u16 udp_port, v3 only |
//!
//! `ROOM_INFO` = u32 room_id, u64 local_comm_id, u16 scene_id, s16 comm_version, u8 node_count_max, u8 security_mode,
//! u8 node_count, node_count x {u8 node_id, u32 vip, 6 B mac, str nickname}, u16 adv_len + adv bytes.
//!
//! ERROR codes: 1 bad proto, 2 build mismatch, 3 password, 4 room full, 5 room not found, 6 version mismatch,
//! 7 not host, 8 rejected, 9 rate limit, 10 bad frame, 11 wrong room code. An ERROR answering HELLO, ERROR 9 and ERROR 10 for a malformed
//! frame close the connection; the others leave it open. The message is plain English for the launcher/game to show; it never carries raw io errors.
//!
//! Protocol 2 = protocol 1 + the room key in SCAN, CREATE and JOIN (and ERROR 11). A protocol 1 client gets ERROR 1 with a
//! message saying to update the game.
//!
//! # Room codes
//!
//! A host may create its room with a room code: 16 symbols of Crockford base32 (`0-9 A-Z` without I L O U, 80 bits from the
//! OS random source), shown as `XXXX-XXXX-XXXX-XXXX`; on input case, `-` and spaces do not matter and O reads as 0, I and L as 1
//! (format from pokopia-native's coop_auth.c). The code never goes over the wire: clients send the room key, the first 16 bytes
//! of SHA-256("pokopia-relay room v1" + the 16 normalized symbols). A key of 16 zero bytes means "no code".
//! A room created with no code is an open room, listed to everyone (LAN play as before). A room created with a key is listed only
//! to SCANs carrying that key and joined only by JOINs carrying it. Wrong codes are counted and limited (see JOIN).
//!
//! # Semantics
//!
//! - HELLO must be the first frame. proto must be 2 or 3 (else ERROR 1); the build stamp must equal the server's when the
//!   server has one (else ERROR 2); the password must equal the server's when it has one (else ERROR 3).
//!   Success: WELCOME. Any other frame before HELLO, or a second HELLO: ERROR 10 + close.
//! - SCAN -> ROOM_LIST of the open rooms plus the rooms whose key equals the SCAN's, that are not full and match the filter. The list is cut so the frame stays
//!   <= 8192 B (a full ROOM_INFO is up to 757 B, 64 rooms would not fit).
//! - CREATE (caller in no room): new room R = lowest free 1..254, subnet 169.254.R.0/24; the caller is its host,
//!   vip 169.254.R.1, node 0. Reply JOINED (v3: then UDP_OFFER) then ROOM_STATE. No free room (64 max): ERROR 4.
//! - Room keys (SCAN, CREATE, JOIN): a connection keeps the first nonzero key it sends; a different nonzero key later counts as
//!   a wrong code (ERROR 11). Keys are compared in constant time.
//! - JOIN (caller in no room): room must exist (5); a room with a key: the JOIN's key must equal it (11, checked before anything
//!   else; no key counts as wrong). 5 wrong codes on one connection: ERROR 9 + close. 30 wrong codes from one IP address in
//!   10 minutes (the first keyed SCAN of a connection that finds nothing counts too): every request with a key from that address
//!   gets ERROR 9 + close until the window ends. Then: the caller not rejected from it (8), not full (4), comm_version
//!   equal (6), passphrase hash equal (3). Member gets vip 169.254.R.N with N = lowest free 2..=8, node id N-1.
//!   Reply JOINED (v3: then UDP_OFFER), then ROOM_STATE to every member (the new one included).
//!   CREATE/JOIN while already in a room: ERROR 10 (connection stays open).
//! - mac of node N in room R = 02:00:a9:fe:R:N. vIPs are u32 in host order (169.254.1.2 = 0xa9fe0102).
//! - LEAVE: LEFT 0 to the caller. A guest leaving: ROOM_STATE to the rest. The host leaving closes the room: LEFT 1 to
//!   every guest. Disconnect and keepalive timeout leave the same way.
//! - ADVERTISE: host only (else ERROR 7); replaces the advertise data, ROOM_STATE to every member.
//! - REJECT: host only (else ERROR 7). The member with that vip gets LEFT 2 and is removed; the same connection cannot
//!   JOIN that room again (ERROR 8). ROOM_STATE to the rest. Unknown vip / the host's own vip: ignored.
//! - DATAGRAM: only from a room member (else dropped). The server sets src_ip to the sender's vip (no spoofing).
//!   dst_ip = 169.254.R.255: copied to every other member. dst_ip = a member's vip: sent to that member only.
//!   Anything else (unknown vip, the sender's own vip, another subnet): dropped silently. The sender gets none of
//!   its own datagrams back; the client loops broadcast/self traffic back locally if the game needs it.
//! - Keepalive: the client sends PING every 5 s (PONG echoes the nonce). A client that sends no frame for 15 s is
//!   dropped (as a disconnect).
//! - Caps: frame 8192 B, 8 nodes/room, 64 rooms, 32 connections (the 33rd gets ERROR 4 "server full" and is closed),
//!   2000 frames/s per client (ERROR 9 + close).
//! - Server stop: LEFT 3 to every room member, then all connections close.
//! - Slow clients: every client has a bounded send queue (256 frames) and a 5 s write timeout. A full queue or a
//!   timed-out write drops that client; nobody else waits on it.
//!
//! Protocol 3 = protocol 2 + the UDP datagram path (UDP_MODE, UDP_OFFER, the UDP packets below). A v2 client is served
//! exactly as before (TCP only, no UDP_OFFER); protocol 1 is refused as in v2. A v3 client that gets ERROR 1 talks to a v2
//! server: it may connect again with proto 2.
//!
//! # UDP datagram path (v3)
//!
//! Head-of-line blocking: on a lossy link one lost TCP segment stalls every datagram behind it. The game sends UDP and
//! has its own resends, so its datagrams may as well be lost or reordered. Design after pokopia-native's coop_mpnet.c
//! (UHELLO/UACK token proof from the control link's address, TCP until then).
//!
//! The server binds UDP on the TCP listener's address and port. When a v3 client enters a room (CREATE, or a JOIN that
//! passed every check, room code included), JOINED is followed by UDP_OFFER with a fresh random u64 token; leaving the
//! room (LEAVE, LEFT, disconnect) kills it. A client in no room, or one that failed a room code, has no token, so the
//! UDP path never reaches past the TCP checks. No UDP_OFFER when UDP is off or could not be bound: the client stays on
//! TCP. UDP packets, little-endian, one per datagram, no length prefix:
//!
//! | type | name  | dir | body after `u8 type, u64 token` |
//! |------|-------|-----|---------------------------------|
//! | 0x01 | HELLO | c->s | - |
//! | 0x81 | ACK   | s->c | - (the token of the HELLO) |
//! | 0x08 | DATA  | c->s | u16 src_port, u32 dst_ip, u16 dst_port, bytes (<= 4096) |
//! | 0x86 | DATA  | s->c | u32 src_ip, u16 src_port, u32 dst_ip, u16 dst_port, bytes (the receiver's own token) |
//!
//! - A packet counts only when it is well formed, its token is live, and it comes from that connection's TCP peer IP;
//!   anything else is dropped without an answer. Valid packets over 2000 a second per token are dropped too.
//! - HELLO: the source address becomes the connection's UDP endpoint (a later HELLO from a new port moves it: NAT
//!   rebinding) and gets ACK, which is as long as the HELLO. The server never sends to an address that has not proven
//!   a token, and never more bytes than it got from it before that, so it cannot be used to amplify.
//! - DATA from the endpoint is routed like DATAGRAM. The two paths mix freely: a client may send over either at any time.
//! - UDP_MODE 1 (after the client got an ACK): the server sends this client's datagrams as UDP DATA to its endpoint;
//!   UDP_MODE 0: back to TCP DATAGRAM frames. Before UDP_MODE 1 everything goes over TCP, so a client whose UDP is
//!   blocked works as in v2. UDP_MODE 1 with no endpoint yet is ignored; UDP_MODE from a v2 client: ERROR 10 + close.
//! - Client side: HELLO every 100 ms until ACK (3 s), then every 5 s (keepalive and NAT binding refresh); no ACK for
//!   15 s while on UDP: UDP_MODE 0 and back to TCP, still probing every 5 s.

mod codec;
mod proto;
pub mod room_code;
mod server;

pub use server::{Config, RelayServer, Status};

/// Default TCP port of the relay.
pub const DEFAULT_PORT: u16 = 39660;
