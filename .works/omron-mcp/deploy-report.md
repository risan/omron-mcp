# Deploy report

## 2026-10-30: first deploy

- Host: CT 103 `public-apps`, beside garmin-mcp (8081) and mfp-mcp (8080).
- Binary: `/usr/local/bin/omron-mcp`, 4,377,000 bytes, sha256 `806a938e…bd31b`, built from tag `v0.1.0` with `scripts/build-release.sh`.
- Service: OpenRC `omron-mcp`, user `omron`, `/var/lib/omron-mcp` (0700), `127.0.0.1:8082`, memory cap 24M/32M, enabled in the default runlevel.
- Public URL: `https://omron.risanb.com` through the existing Cloudflare tunnel.
- The owner ran `set-password` and `login` (country ID). The first sync cached 4 readings.
- Checks: `/healthz` 200, `/` 200, `/.well-known/oauth-protected-resource` 200, `POST /mcp` without a token 401. RSS 4,292 kB after start.
- `omron-connect` 0.1.0 published on crates.io by the owner.
