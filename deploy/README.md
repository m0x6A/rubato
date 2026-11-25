# deploy

Rubato's stack as it runs on the apps host (`/opt/stacks/rubato`, Dockge). A deploy copies
`compose.yaml` and `livekit.yaml` there, sets the `app` image tag, and runs
`docker compose up -d --pull never`. Secrets live only in the stack's `.env` on the host.
How a deploy goes is in `.claude/profiles/dockge.md`.

| Service | Runs | What |
| --- | --- | --- |
| `app` | always | rubato-server with the web |
| `livekit` | with the `voice` profile | LiveKit for a plan room's voice channel (ADR 0073, #315) |

`pnpm deploy:check` checks these files and `pnpm deploy:test` makes the check fail on purpose
on `fixtures/bad/`.

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

## Speech on Azure AI Foundry

The speech provider (ADR 0073) for agents to hear and answer in a voice channel: one Foundry
account, `rubato-speech` in resource group `rubato` on the developer subscription, declared in
`azure/speech.bicep`. It holds `gpt-4o-mini-transcribe` (GlobalStandard: may be processed
outside the EU) and `tts` (Standard, in Sweden Central). Rubato calls it over the
OpenAI-compatible v1 API, `https://rubato-speech.openai.azure.com/openai/v1`.

Create it (a person runs this; it changes nothing on the host):

```sh
az group create -n rubato -l swedencentral
az deployment group what-if -g rubato -f deploy/azure/speech.bicep
az deployment group create -g rubato -f deploy/azure/speech.bicep
```

Add to the stack's `.env`:

```sh
RUBATO_SPEECH=openai_compatible
RUBATO_STT_URL=https://rubato-speech.openai.azure.com/openai/v1
RUBATO_TTS_URL=https://rubato-speech.openai.azure.com/openai/v1
RUBATO_STT_MODEL=gpt-4o-mini-transcribe   # the deployment name
RUBATO_TTS_MODEL=tts
RUBATO_TTS_VOICE=alloy
RUBATO_STT_API_KEY=<the account key>
RUBATO_TTS_API_KEY=<the same key>
# RUBATO_SPEECH_LANGUAGE=en               # optional; the model detects it otherwise
```

The key goes from Azure to the host over stdin, never printed and never on a command line:

```sh
k=$(az cognitiveservices account keys list -n rubato-speech -g rubato --query key1 -o tsv)
printf 'RUBATO_STT_API_KEY=%s\nRUBATO_TTS_API_KEY=%s\n' "$k" "$k" | ssh <host> 'cat >> /opt/stacks/rubato/.env'
unset k
```

Then `docker compose up -d --pull never`. The consent panel names
`rubato-speech.openai.azure.com`.

Limits and cost:
- TTS is capped at 3 requests a minute: the region's whole quota. A fourth spoken reply in a minute
  is refused (429) and counts as a failed `rubato.voice.speech{rubato.voice.op="tts"}`.
- Transcription is about $0.003 a minute of speech. The subscription's monthly credit caps spend.
- Azure may keep what it is sent for abuse monitoring for up to 30 days.

Turn it off: remove the `RUBATO_SPEECH`, `RUBATO_STT_*` and `RUBATO_TTS_*` lines and run `up`
again; people still hear each other. Remove it: `az group delete -n rubato`. Rotate the key:
`az cognitiveservices account keys regenerate -n rubato-speech -g rubato --key-name key1`, then
move it as above.

At 3 am: `rubato.voice.speech{rubato.voice.op, rubato.outcome}` and the `SpeechProviderFailing`
alert. In the server log, 401 means the key in `.env` is not the account's, 429 the quota, 404 a
wrong URL or deployment name.

## At 3 am

The Voice panel on the Rubato overview counts `rubato.voice.join_failed{rubato.reason}` from
browsers. `signal` means the https proxy or 7880; `ice` means 7881/7882 are blocked or
`LIVEKIT_NODE_IP` is wrong. Then `docker compose ps livekit` (health check on
`http://127.0.0.1:7880/`) and `docker compose logs livekit`. If LiveKit's log says
`token signature is invalid`, the keys in `.env` changed and only one of the two services was
recreated: run `docker compose up -d --pull never` again.

Setup notes for LiveKit, TURN and speech providers: `docs/plans/146-voice-meetings/design.md` §9.
