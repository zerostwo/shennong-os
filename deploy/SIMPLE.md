# Three-image deployment

The default deployment has exactly three long-running containers and three
Docker Hub repositories:

- `zerostwo/shennong-os`: WebUI, control-plane API, Agent Runtime, gateway, and
  the OS metadata database;
- `zerostwo/shennong-db`: governed biomedical data plane and its storage
  services;
- `zerostwo/shennong-runtime`: Runtime daemon plus the batch, RStudio, and
  Jupyter workload environment.

No secret files need to be created manually. On first start, Shennong OS writes
the service credentials and Runtime Ed25519 key pair to the shared `config`
directory. DB and Runtime wait for those files without logging their contents.

```bash
mkdir shennong && cd shennong
curl -fsSLO https://raw.githubusercontent.com/zerostwo/shennong-os/main/deploy/compose.yaml
docker compose pull
docker compose up -d --wait
```

Open <http://localhost:18081>. The one-time administrator bootstrap token is
available only from the local config volume:

```bash
docker compose exec shennong-os sh -c 'cat /config/os-bootstrap-token'
```

Persistent state defaults to `./shennong-data`. Only the WebUI port is
published, and it binds to `127.0.0.1` by default. Copy `.env.example` to `.env`
only when changing the port, data directory, browser origins, Docker socket, or
pinning the three images by digest.

## Security modes

This small Compose file uses Runtime `simple` mode and the host Docker socket.
That socket gives the Runtime container host-level Docker control. Use this
mode only on a trusted single-user host where simplified operation is the
priority; do not expose the Runtime port.

The existing `shennong-runtime/deployments/docker/compose.rootless.yaml`
remains the hardened production profile. It uses the same single
`zerostwo/shennong-runtime` image but keeps workloads on a dedicated rootless
Docker daemon with the nftables attestation guard.

For a public hostname, configure distinct product and IDE origins:

```dotenv
SHENNONG_PUBLIC_ORIGIN=https://shennong.example.com
SHENNONG_IDE_PUBLIC_ORIGIN=https://ide.shennong.example.com
SHENNONG_IDE_HOST=ide.shennong.example.com
SHENNONG_OS_COOKIE_SECURE=true
```

Terminate TLS in a reverse proxy and forward the product and IDE hostnames to
`127.0.0.1:18081`.
