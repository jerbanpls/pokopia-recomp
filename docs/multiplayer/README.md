# Multiplayer (Link Play) — experimental

Pokopia's **Link Play** (local wireless, two players on one island) running between two PCs through the relay server.
First working session on 2026-11-03: a desktop PC hosting and a Legion Go S (Proton) visiting.

![Two players on one island](linkplay-1.webp)
![The same session from the other view](linkplay-2.webp)

## How it works

The game thinks it is on local wireless. The runtime's `ldn` service turns its networks into rooms on a relay server, and its
`bsd` UDP sockets carry the game's packets through that server on a virtual 169.254.R.N/24 network. The game code above `nn::ldn`
is untouched.

    pkp.exe (host) --TCP+UDP--> relay server <--TCP+UDP-- pkp.exe (guest)

The TCP connection carries the lobby (rooms, joins, keepalive). The game's packets go over UDP to the same port once the relay
has checked the player's UDP address; when UDP is blocked they stay on the TCP connection, as before. On a lossy link (Wi-Fi,
mobile, long distance) UDP avoids one lost packet holding up every packet behind it. `[net] udp = off` keeps a player on TCP.

## Trying it

The launcher has a **Link Play** page that does steps 1 and 2 below for you: pick Alone, Host on this PC (starts the relay, shows the
address and a room code) or Visit (type the host address and code). It writes `[net] server` and `room_code` in pkp.ini.

1. Start a relay server on one PC: `./PokopiaLauncher --host` (port 39660; `--port N` to change it; `--no-udp`: TCP only).
   Other PCs on the LAN need that port reachable (firewalld: `sudo firewall-cmd --add-port=39660/tcp --add-port=39660/udp`). A relay behind a router
   needs TCP **and** UDP 39660 forwarded (TCP alone works, without the UDP benefit).
2. On every player's PC, `pkp.ini`:

       [net]
       server = <relay PC's IP>:39660

   For a relay that other people use too (or one reachable from the internet), the host adds `room_code = new` under `[net]`:
   the game makes a code (`XXXX-XXXX-XXXX-XXXX`), logs it and writes it to `room-code.txt` in the game folder. The visitors
   put that code in their own `[net] room_code`. A room made with a code is listed only to players with the same code. Without
   `room_code` everything works as before: rooms are open to everyone on the relay. The relay and every player need a build
   with room codes (relay protocol 2); an older game is refused with a message saying to update it.

3. In game: the PC in your first house -> Link Play (unlocked after Environment Level 2 in Withered Wasteland). One player picks
   **Invite Others to Visit**, the other **Visit a Friend**.

**Each player needs their own save.** The game gives every save a player UUID when the game is started, and the host refuses a
visitor whose UUID it already has (error 9005, "Communication error"). Two copies of one save cannot play together.

## Status

Joining and walking around together work. Not yet tested: long sessions, more than two players, leaving and rejoining, playing
over the internet. Expect bugs; report them with both players' `pkp.log`.
