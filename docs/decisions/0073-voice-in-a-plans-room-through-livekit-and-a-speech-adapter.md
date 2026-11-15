# 0073. A plan's room may have a voice channel on a self-hosted LiveKit, and speech reaches agents only as room text through a speech adapter, behind a flag that is off by default

- **Status:** proposed
- **Date:** 2026-11-15
- **Decided by:** agent (Claude Code, m0x6A/rubato#146), drafted at Mattias Jonsson's request of 2026-11-15 that #146 get a superseding ADR, built behind a flag that is off by default and not deployed; the provider adapter shape is a best-effort agent choice under his delegation; a human accepts it
- **Seam:** web-server, web-livekit, server-speech
- **Issue:** m0x6A/rubato#146

## Context

AGENTS.md's v0 scope says Rubato "does not: … Audio". #146 asks for a voice meeting in a plan's
room that a person can call their agent into, built on an open-source meeting stack rather than a
media server of our own. It also asks that everything ADR 0057 decides about rooms still holds:
the owner calls their own agent in, the agent answers only when addressed, and it leaves when
dismissed or idle. Claude Code stays the only agent (ADR 0003). The page's Content-Security-Policy
(ADR 0066) allows connections only to Rubato's own origin. Mattias decided on 2026-11-15 that
#146 gets this superseding ADR and is built behind a flag that is off by default, up to a merged
PR and not deployed. Plan: `docs/plans/146.md`; design: `docs/plans/146-voice-meetings/design.md`.

## Decision

**Voice is a transport on the room, not a new agent.**
- A plan's room (ADR 0057) may have a voice channel. People hear each other through a LiveKit
  server that the team runs itself, beside Rubato.
- Rubato never starts, supervises or connects to LiveKit. It only mints a LiveKit access token for
  a logged-in person, signed HS256 with the LiveKit secret:
  - name: the login; identity: the login plus a random suffix, so two tabs do not evict each
    other;
  - one LiveKit room per plan room;
  - microphone only.
- The token's `exp` is 10 minutes, and it bounds only the join. LiveKit refreshes the tokens of
  connected participants, and Rubato cannot remove anyone from a call. A person whose login is
  revoked (ADR 0022) stays in a voice call they are already in until they leave or LiveKit's keys
  change. The design says so.
- The web joins with `livekit-client`, loaded only when voice is on.
- With voice on, the page's `connect-src` also allows the LiveKit origin from
  `RUBATO_LIVEKIT_URL`, its `wss:` and `https:` forms. With voice off the policy is exactly as ADR
  0066 says.

**Speech becomes room text; replies to spoken questions become speech.**
- A person speaks by holding *Talk* (to the room) or *Ask <handle>* (to an agent in the room).
- The browser sends that one utterance to Rubato, with an id it chose, so a repeat posts once.
  Rubato checks the room and the agent first, then transcribes it through a speech adapter. It
  posts the text in the room as that person, marked `spoken`, through the rooms slice's own post
  path.
- *Ask <handle>* prefixes the agent's handle, so the existing addressing makes the turn.
- When an agent answers a turn whose trigger was `spoken`, each browser in the voice channel plays
  it. Rubato speaks a spoken-length cut of the reply through the same adapter: code removed, at
  most 400 characters, at a sentence end. The full reply stays in the room thread.
- Replies to typed turns are never sent to the speech provider.
- The agent never touches audio. It gets text through its owner's companion, exactly as for a
  typed room turn (ADR 0057), so ADR 0004 holds.

**The speech adapter.** One port, `Speech`, with `transcribe(audio, mime, language?)` and
`synthesize(text)`. It has two adapters:
- `openai_compatible`: `POST {stt}/audio/transcriptions` and `POST {tts}/audio/speech`, with
  separate base URLs for STT and TTS. This one shape covers local servers (Speaches,
  faster-whisper-server, whisper.cpp's server with an OpenAI path, Kokoro-FastAPI) and hosted
  APIs. The provider is configuration, not code.
- `fake`: for tests and the dev server only. Transcription returns the audio bytes read as UTF-8,
  and synthesis returns the text as bytes.

Calls to the provider are bounded:
- a deadline of 30 s each;
- at most 2 at a time; a third gets 503 `busy`;
- a response body of at most 4 MiB.

**No audio is stored by Rubato.**
- An utterance is held in memory for its transcription and dropped. It is at most 60 s and 2 MiB.
- A synthesized reply is kept in a bounded in-memory cache, at most 32 clips and 10 minutes each,
  so every listener gets the same clip from one synthesis. It is served from Rubato's own origin
  and is never written to disk or to the database.
- The transcript is room text, kept as room messages are (ADR 0057). It is never posted to
  GitHub.
- A hosted provider may keep what it is sent under its own terms (for example, some keep uploads
  for up to 30 days). The consent text says so when the provider is not on the team's own
  machines.

**Consent is shown.** Before a browser's first utterance, the voice panel says that two things go
to the provider, whose host it names, and through the agents to Anthropic:
- the person's speech, which is transcribed and posted in the room;
- agents' replies to spoken questions, which are spoken.

The person confirms once per browser. While joined, the panel shows "speech via <host>".

**The flag.**
- `RUBATO_VOICE` is off by default. Off, every voice route answers 404 `voice_off`, the CSP is
  unchanged, and the web shows no voice panel.
- On, a voice channel needs `RUBATO_LIVEKIT_URL`, `RUBATO_LIVEKIT_API_KEY` and
  `RUBATO_LIVEKIT_API_SECRET`. Speech needs `RUBATO_SPEECH` (`openai_compatible` or `fake`) and its
  settings.
- With voice on and no speech adapter, people can talk to each other and no agent hears them.

**What stays as it was.**
- An agent is in a room only when its owner called it in, and answers only when addressed.
- A spoken question is a room turn: it counts against the owner's cap and the room's turn limit
  like any other.
- Rubato runs no agent and no media server.
- Nothing listens on a developer's machine.
- The durable record is the plan and its decisions.

## Alternatives considered

- **A bridge participant in the LiveKit room (LiveKit Agents, or the Rust SDK)**: it would hear the meeting without push-to-talk, but it means a second long-running process, a WebRTC stack and voice-activity detection. It is the next slice, with its own ADR.
- **Voice on the direct chat only (#146 slice 1)**: no shared meeting.
- **Jitsi Meet**: a bot joins through a headless browser, and the UI is an iframe beside ours.
- **A speech-to-speech realtime model as the participant**: a different agent from the owner's session, against ADR 0003.
- **The companion joins the meeting**: puts a media stack on every developer machine.
- **One adapter per provider**: more code for the same two calls; the OpenAI-compatible shape is what local and hosted servers share.
- **Speak every agent reply in the room**: it sends typed context to a provider nobody consented to, and costs a synthesis per reply.
- **Store audio for replay**: the issue says audio is not stored.

## Consequences

Easier: a team can talk about a plan with its agent in the room, and a question asked aloud gets a
spoken answer plus the full reply in the thread.

Harder:
- Rubato now depends on a LiveKit host. That host needs UDP or TURN, and HTTPS, since browsers
  give microphone access only in a secure context.
- A speech provider sees what is said. A hosted one takes it off the team's machines; the consent
  text names it.
- Push-to-talk is less natural than an open meeting.
- Spoken answers depend on the agent putting the answer first. The voice turn asks it to.
- Rubato cannot end anyone's call.

O-rows:
- server-speech: spans `rubato.voice.stt` and `rubato.voice.tts`,
  `rubato.voice.speech{rubato.voice.op, rubato.outcome}`, alert `SpeechProviderFailing`;
- web-livekit: `rubato.voice.tokens{rubato.outcome}`, and join failures reported by the browser
  as `rubato.voice.join_failed{rubato.reason}`;
- web-server: `rubato.web.rejected{rubato.route, rubato.reason}` for the voice routes.

On acceptance:
- ADR 0066's status becomes "accepted; CSP extended in part by 0073";
- AGENTS.md's v0 "Does not: Audio" reads "Audio, except a plan's room voice channel behind the
  off-by-default voice flag (ADR 0073); Rubato stores no audio".

## Supersedes

In part, and only while the voice flag is on:

- AGENTS.md's v0 non-goal "Audio", which no ADR recorded.
- **0066:** "Nothing else is allowed from anywhere else" gains the LiveKit origin in `connect-src`.
  The rest of 0066 stands: the same-origin check on writes, `frame-ancestors 'none'`, and
  `nosniff`.

ADR 0057 stands unchanged. Voice adds a way to post in a room and a way to hear a reply; who calls
an agent in, when it answers and when it leaves stay as 0057 says. ADR 0003 stands: no speech
model is an agent.
