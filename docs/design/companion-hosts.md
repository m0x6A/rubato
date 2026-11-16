# Where a companion runs, and what each way assumes

ADR 0076, plan 134 design §1 and §6. Every way of running `rubato-companion` keeps the same rules:
- it dials out to the server and listens on nothing but its loopback hook port (ADR 0004, ADR 0071);
- it is paired by device code with one person, who owns it and everything it launches (ADR 0011,
  ADR 0062);
- its state dir and config are private to its OS user;
- Claude Code is the only agent it runs (ADR 0003).

| Way | Mode | Needs | Good for |
| --- | --- | --- | --- |
| On your machine | `interactive` (the default) | tmux 3.7 or newer, git, `claude` logged in, the `rubato` skill in `~/.claude/skills` | your own sessions in your own tmux, which you can attach to |
| On a machine, as a service | `headless` | git, `claude` (logged in, or an API key), the skill | agents that run unattended, one `claude -p` turn at a time |
| In a container | `headless` only | Docker or Podman; the image brings git, Node and Claude Code | the same, without installing anything on the host |

## What today assumes

Until #134, the docs and every issue assumed the first row only:
- one person at a terminal, in tmux;
- with `claude` installed and logged in under their own account.

That is still the default and the only interactive path. A headless companion (ADR 0062) needs no
terminal. Both are in the same one-owner model: nothing here lets anyone but the owner start,
message or stop a session. A person's own lead agent (ADR 0072, behind the orchestrator flag) is
still that person's.

## Claude through an API

"The API version of Claude" is Claude Code authenticated by an API key, not a different agent. The
companion passes its environment to every `claude` it starts, so set one of these where the
companion runs:

| Set | Kind the companion announces |
| --- | --- |
| `ANTHROPIC_API_KEY` or `ANTHROPIC_AUTH_TOKEN` | `api_key` |
| `CLAUDE_CODE_USE_BEDROCK=1` and the AWS credentials | `bedrock` |
| `CLAUDE_CODE_USE_VERTEX=1` and the Google credentials | `vertex` |
| an `apiKeyHelper` in `~/.claude/settings.json` | `api_key` |
| none of these | `login` |

The companion announces only the kind (`claude_auth:api_key`), never a value, and the lane shows
*API key*, *Bedrock* or *Vertex*: spend is then real money per token, so check the plan's spend cap.

- Put the key in a file only you can read (`chmod 600`), never on a command line.
- The label describes the companion's own environment. In interactive mode a session's pane
  inherits the tmux server's environment, which can differ if tmux was started earlier.

## As a service

`~/.config/systemd/user/rubato-companion.service`:

```ini
[Unit]
Description=Rubato companion (headless)
After=network-online.target

[Service]
ExecStart=%h/.cargo/bin/rubato-companion run
# ANTHROPIC_API_KEY=... and nothing else; mode 0600.
EnvironmentFile=%h/.config/rubato/companion.env
Restart=on-failure
RestartSec=10

[Install]
WantedBy=default.target
```

Then:
1. set `companion.mode: headless` in `~/.config/rubato/local.yml`;
2. run `rubato-companion pair --server <url>` once, by hand;
3. run `systemctl --user enable --now rubato-companion`;
4. to keep it running after you log out, run `loginctl enable-linger`.

On macOS, a launchd agent does the same,
`~/Library/LaunchAgents/se.rubato.companion.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>se.rubato.companion</string>
  <key>ProgramArguments</key><array><string>/usr/local/bin/rubato-companion</string><string>run</string></array>
  <key>KeepAlive</key><true/>
  <key>RunAtLoad</key><true/>
</dict></plist>
```

launchd has no environment file, so use an `apiKeyHelper` in `~/.claude/settings.json` for the
key.

## In a container

`Dockerfile.companion` builds a headless companion with git, Node and Claude Code at a pinned
version, running as user `rubato` (uid 1000). One volume, `/home/rubato`, holds everything it
keeps: the config, the state dir, the clones and the worktrees.

```sh
docker build -f Dockerfile.companion -t local/rubato-companion:dev .
# local.yml into the volume, with companion.mode: headless and the repos cloned under the volume
docker run -it --rm -v rubato-home:/home/rubato local/rubato-companion:dev rubato-companion pair --server https://rubato.example
docker run -d --name rubato-companion -v rubato-home:/home/rubato --env-file companion.env local/rubato-companion:dev
```

- `rubato-companion run` refuses `mode: interactive` in the image, since there is no tmux in it.
- There is no Docker-in-Docker and no Docker socket: a worker's tools run in the same container.
  A container per worker is a later step with its own ADR.
- The volume holds the clones, the worktrees and the pairing token, and the env file holds the key.
  Treat both like a developer's home directory.
- CI does not build or publish this image. Build it yourself, and bump `CLAUDE_CODE_VERSION`
  when you want a newer Claude Code.

## A lead agent (ADR 0072)

Behind the orchestrator flag, on the server (`RUBATO_ORCHESTRATOR=1`) and on the companion
(`companion.orchestrator: true`), a person may start a lead agent that starts, messages and
stops the workers it launches for them, and nobody else's.

Within one OS user, any of your sessions on that companion could read the lead's files and act as
the lead. The harm stays within your own sessions and caps, and ADR 0072 accepts this. For more
isolation, run the lead on its own companion: another OS user, or another container.
