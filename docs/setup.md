# Setup: omron-mcp on an existing Alpine LXC with a Cloudflare tunnel

This adds omron-mcp to an Alpine LXC that already runs a service behind a `cloudflared`
tunnel (the `public-apps` LXC of mfp-mcp). It needs no second tunnel and no second
`cloudflared`. If you have no such LXC yet, follow the mfp-mcp setup guide first: it covers
creating the LXC, installing `cloudflared`, `ca-certificates` and `cgroup-delegate`.

omron-mcp listens on `127.0.0.1:8082` (mfp-mcp uses 8080, garmin-mcp 8081).

## 1. Build and copy the binary

On the dev box (needs rustup with the musl target, zig and `cargo install --locked
cargo-zigbuild`):

```sh
scripts/build-release.sh
scp target/x86_64-unknown-linux-musl/release/omron-mcp \
    deploy/alpine/omron-mcp.initd deploy/alpine/omron-mcp.confd root@<lxc-ip>:/root/
```

On the LXC:

```sh
install -m 0755 /root/omron-mcp /usr/local/bin/omron-mcp
install -m 0755 /root/omron-mcp.initd /etc/init.d/omron-mcp
install -m 0644 /root/omron-mcp.confd /etc/conf.d/omron-mcp
```

## 2. Create the service user and the data directory

```sh
addgroup -S omron
adduser -S -D -H -G omron -h /var/lib/omron-mcp -s /sbin/nologin omron
install -d -o omron -g omron -m 0700 /var/lib/omron-mcp
```

`state.json` will hold your OMRON tokens and the OAuth token hashes, and `readings.json`
your blood pressure readings. Treat the directory like a password file and include it in no
shared backup.

Edit `/etc/conf.d/omron-mcp` and set `OMRON_MCP_PUBLIC_URL` to your public origin
(`https://omron.example.com`, no path, no trailing slash). Tokens are tied to this URL. The
file also holds the memory cap of the service (`rc_cgroup_settings`: 24 MB soft, 32 MB
hard), which needs `cgroup-delegate` from the mfp-mcp guide.

## 3. Set the owner password

The consent page asks for this password when Claude connects. Use a long passphrase.

```sh
. /etc/conf.d/omron-mcp
omron-mcp set-password
```

It asks twice without echo and needs at least 12 characters. Changing it revokes every OAuth
token, so you reconnect from Claude.

## 4. Sign in to OMRON connect

```sh
. /etc/conf.d/omron-mcp
omron-mcp login
```

It asks for your country code (for example `ID`), the email of your OMRON connect account
and, without echo, its password. The password is used once and never stored; the server keeps
the OMRON tokens in `state.json`. The command then runs the first sync, which downloads your
whole history and can take a few minutes. The OMRON phone app and the server can stay signed
in at the same time.

Run `omron-mcp status` to see the result. A country OMRON serves from no cloud is refused.
Accounts on the v2 cloud (Europe, the Americas) are untested.

## 5. Start the service

```sh
rc-update add omron-mcp default
rc-service omron-mcp start
curl http://127.0.0.1:8082/healthz      # prints: ok
```

Logs are in `/var/log/omron-mcp/`. They contain no tokens or passwords.

## 6. Add the hostname to the existing tunnel

No second `cloudflared` and no second tunnel: in the Cloudflare dashboard open the tunnel of the LXC (Zero Trust -> Networks -> Tunnels),
**Public hostname -> Add**, and point a new hostname (`omron.example.com`) at
`http://127.0.0.1:8082`. Do **not** put a Cloudflare Access application in front of it:
Claude authorizes through the server's own OAuth flow.

## 7. Add the connector in Claude

1. In claude.ai open **Settings -> Connectors -> Add custom connector**.
2. Name: `OMRON`. URL: `https://omron.example.com/mcp`. Add, then **Connect**.
3. Enter the owner password from step 3 and approve.

Connectors added on the web are available in the iPhone app with the same account. Test it
with:

> What was my average blood pressure over the last week?

Claude should call `omron_get_blood_pressure` and answer from your readings.

## Everyday operations

| Task | Command |
|---|---|
| See the state (no secrets) | `omron-mcp status` |
| The tool says the OMRON login expired | `omron-mcp login` (step 4); the cache is kept |
| Forget every connected Claude client | `omron-mcp revoke-tokens` |
| Change the owner password | `omron-mcp set-password` (this also disconnects Claude) |
| Update the server | copy the new binary to `/usr/local/bin/omron-mcp`, then `rc-service omron-mcp restart` |

Run them with the settings from `/etc/conf.d/omron-mcp` loaded (`. /etc/conf.d/omron-mcp`),
otherwise they look at the default data directory. They work while the service runs.

Signing in as another account empties the readings cache first; signing in again as the same
account keeps it.
