# deploy

Rubato's stack as it runs on the apps host (`/opt/stacks/rubato`, Dockge). A deploy copies
`compose.yaml`, `livekit.yaml` and `Caddyfile` there, sets the `app` image tag, and runs
`docker compose up -d --pull never`. Secrets live only in the stack's `.env` on the host.
How a deploy goes is in `.claude/profiles/dockge.md`.

| Service | Runs | What |
| --- | --- | --- |
| `app` | always | rubato-server with the web |
| `livekit` | with the `voice` profile | LiveKit for a plan room's voice channel (ADR 0073, #315) |
| `caddy` | with the `public` profile | TLS for a server on the internet, certificates by ACME (ADR 0090) |

`pnpm deploy:check` checks these files and `pnpm deploy:test` makes the check fail on purpose
on `fixtures/bad/`.

## The first deploy with tenants (#345)

The image built from the #345 stack changes how the server starts and who gets in. In order:

1. Stop the app and copy the database, `compose.yaml` and `.env` first. Migrations 0032 to
   0037 rebuild about forty tables and are not additive: a rollback is the previous image tag
   **and** the copied database.
2. In `.env`, before the new image starts: `RUBATO_ALLOW_HTTP=1` on a LAN (below), and
   `RUBATO_OPERATORS=<GitHub user id>`. The first start makes tenant `default` from
   `RUBATO_GITHUB_APP_INSTALLATION_ID`, makes everyone with a session in the last 30 days a
   member, and makes an operator among them its admin. With no operator the tenant has no
   admin and nobody can invite.
3. Copy `Caddyfile` beside `compose.yaml`, also on a LAN: the compose file names it.
4. Start, and read the log for `tenant seeded from the server as it was` and each
   `seed left out login` (invite those people again on **Members**). `tenant default has no
   admin` means step 2 was missed: name one on **Operator**.
5. Open pages must reload: the API moved under `/api/t/<tenant>/…` (ADR 0089).
6. Rebuild each companion from the same commit before it reconnects: the context graph's
   source keys changed, and an older companion drops the sources it no longer knows.

## On a LAN, over plain http

Since ADR 0090 the server refuses to start off loopback when `RUBATO_PUBLIC_URL` is not https.
A LAN deployment says that plain http is meant, in the stack's `.env`:

```
RUBATO_ALLOW_HTTP=1
```

Without that line the container exits at start with `refusing to start: RUBATO_PUBLIC_URL … is
not https`. With it nothing else changes: no secret is required that was not required before.

## On the internet

A server anyone can reach runs behind Caddy, which gets its certificate by ACME. The host needs
a DNS name pointing at it and ports 80 and 443 open. In the stack's `.env`:

```
COMPOSE_PROFILES=public
RUBATO_DOMAIN=rubato.example.org
RUBATO_ACME_EMAIL=you@example.org
RUBATO_PUBLIC_URL=https://rubato.example.org
RUBATO_APP_BIND=127.0.0.1
RUBATO_TRUSTED_PROXY=172.31.250.0/24
RUBATO_SESSION_SECRET=<at least 32 bytes: openssl rand -hex 32>
RUBATO_TOKEN_KEY=<64 hex digits: openssl rand -hex 32>
RUBATO_OPERATORS=<your GitHub user id; gh api user --jq .id>
```

and the GitHub App's settings as before (`RUBATO_GITHUB_APP_ID`, its key, `GITHUB_CLIENT_ID`,
`GITHUB_CLIENT_SECRET`). `RUBATO_ALLOW_HTTP` must not be set. The server names what is missing
and does not start: `refusing to start: a server off loopback needs its secrets (ADR 0090): …`.

- `RUBATO_APP_BIND=127.0.0.1` keeps the app's own port off the public interface: only Caddy
  reaches it.
- `RUBATO_TRUSTED_PROXY` is the `edge` network of `compose.yaml`. The app reads
  `X-Forwarded-For` only from an address in it, so rate limits count each visitor, not Caddy.
  Left unset, every visitor shares one bucket and the demo answers 429 under little load.
- The first person in is an operator (`RUBATO_OPERATORS`): they log in, open **Operator**,
  create a tenant, attach the App's installation and name its first admin.
- On GitHub, the App's callback URL is `https://<domain>/auth/github/callback` and, for webhooks
  (ADR 0074), its webhook URL `https://<domain>/webhooks/github` with `RUBATO_GITHUB_WEBHOOK_SECRET`
  set to the same secret.

What a visitor meets when a limit is hit: 429 with `Retry-After` (30 anonymous requests at once
per address on login and pairing, then one a second; 60 writes at once per session, then two a
second), and 503 `timeout` for a request that runs past 30 s. Each is counted on
`rubato.web.rejected` with the route's template.

## Voice: turn on

Needs https first (#276): browsers give the microphone only to a secure page, and the page
reaches LiveKit as `wss://`. The https setup must also give LiveKit a name that proxies to
`127.0.0.1:7880` with WebSocket upgrade.

Add to the stack's `.env`:

```sh
COMPOSE_PROFILES=voice                     # starts the livekit service
RUBATO_VOICE=1                             # the server's voice routes and the web's panel
RUBATO_LIVEKIT_URL=wss://<LiveKit's https name>
RUBATO_LIVEKIT_API_KEY=<a key name>
RUBATO_LIVEKIT_API_SECRET=<at least 32 characters, e.g. openssl rand -hex 32>
LIVEKIT_NODE_IP=192.168.50.49              # the address browsers reach the host at
```

Then `docker compose up -d --pull never`. LiveKit reads the same key pair Rubato signs join
tokens with, so the two cannot drift apart. Without a speech provider (`RUBATO_SPEECH`, ADR
0073) people hear each other and no agent hears them.

Open on the LAN: **7881/tcp** (ICE over TCP) and **7882/udp** (all media). 7880 listens on
loopback only.

## Voice: turn off

Remove those lines from `.env`, then:

```sh
docker compose --profile voice stop livekit
docker compose --profile voice rm -f livekit
docker compose up -d --pull never
```

A plain `up` with the profile unset leaves a running `livekit` container where it is. Off,
every voice route answers 404 `voice_off` and the page's CSP is as ADR 0066 says.

## Azure AI Foundry

One Foundry account for the models Rubato calls, `rubato-ai` in resource group `rubato` on the
developer subscription, declared in `azure/foundry.bicep`. Today it is the speech provider (ADR
0073), for agents to hear and answer in a voice channel: `gpt-4o-transcribe` and
`gpt-4o-mini-tts`, both GlobalStandard, in East US 2, the one region with both current speech
models. Speech leaves the EU; the round trip adds about 0.1 s to a turn that waits on the agent
anyway.

Speech goes to the OpenAI-compatible v1 API, `https://rubato-ai.openai.azure.com/openai/v1`,
except transcription: v1 answers it with 404 `DeploymentNotFound`, so it goes to the
deployment's own URL with an `api-version` query (the server keeps a query on a base URL, #319).

Create it (a person runs this; it changes nothing on the host):

```sh
az group create -n rubato -l eastus2
az deployment group what-if -g rubato -f deploy/azure/foundry.bicep
az deployment group create -g rubato -f deploy/azure/foundry.bicep
```

Add to the stack's `.env`:

```sh
RUBATO_SPEECH=openai_compatible
RUBATO_STT_URL=https://rubato-ai.openai.azure.com/openai/deployments/gpt-4o-transcribe?api-version=2025-03-01-preview
RUBATO_TTS_URL=https://rubato-ai.openai.azure.com/openai/v1
RUBATO_STT_MODEL=gpt-4o-transcribe        # the deployment name
RUBATO_TTS_MODEL=gpt-4o-mini-tts
RUBATO_TTS_VOICE=alloy                     # or ash, coral, echo, nova, sage, shimmer, …
RUBATO_STT_API_KEY=<the account key>
RUBATO_TTS_API_KEY=<the same key>
# RUBATO_SPEECH_LANGUAGE=en               # optional; the model detects it otherwise
```

The key goes from Azure to the host over stdin, never printed and never on a command line:

```sh
k=$(az cognitiveservices account keys list -n rubato-ai -g rubato --query key1 -o tsv)
printf 'RUBATO_STT_API_KEY=%s\nRUBATO_TTS_API_KEY=%s\n' "$k" "$k" | ssh <host> 'cat >> /opt/stacks/rubato/.env'
unset k
```

Then `docker compose up -d --pull never`. The consent panel names
`rubato-ai.openai.azure.com`.

Limits and cost:
- Each deployment takes 50 thousand tokens a minute, far more than a room uses. Raise
  `capacity` in the Bicep file if `rubato.voice.speech` shows 429s.
- Transcription is about $0.006 a minute of speech and a spoken reply about a cent. The
  subscription's monthly credit caps spend.
- Azure may keep what it is sent for abuse monitoring for up to 30 days.

Turn it off: remove the `RUBATO_SPEECH`, `RUBATO_STT_*` and `RUBATO_TTS_*` lines and run `up`
again; people still hear each other. Remove it: `az group delete -n rubato`. Rotate the key:
`az cognitiveservices account keys regenerate -n rubato-ai -g rubato --key-name key1`, then
move it as above.

At 3 am: `rubato.voice.speech{rubato.voice.op, rubato.outcome}` and the `SpeechProviderFailing`
alert. In the server log, 401 means the key in `.env` is not the account's, 429 the quota, 404 a
wrong URL or deployment name. Probing by hand with curl: East US 2
drops OpenSSL's TLS 1.3 handshake (curl prints 000), so add `--tls-max 1.2`; the server's rustls
is not affected.

## At 3 am

The Voice panel on the Rubato overview counts `rubato.voice.join_failed{rubato.reason}` from
browsers. `signal` means the https proxy or 7880; `ice` means 7881/7882 are blocked or
`LIVEKIT_NODE_IP` is wrong. Then `docker compose ps livekit` (health check on
`http://127.0.0.1:7880/`) and `docker compose logs livekit`. If LiveKit's log says
`token signature is invalid`, the keys in `.env` changed and only one of the two services was
recreated: run `docker compose up -d --pull never` again.

Setup notes for LiveKit, TURN and speech providers: `docs/plans/146-voice-meetings/design.md` §9.
