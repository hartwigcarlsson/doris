# doris-cli

The command line for Doris, for people and AI agents. It talks to the same
gRPC-Web API as the web app, with an API token. The help texts are Swedish;
the JSON field names and error codes are English.

## Install

```
cargo install --path crates/cli     # or: make dist  ->  target/dist/doris-cli
```

## Settings

| Variable | |
|---|---|
| `DORIS_TOKEN` | API token (required). Never printed or logged. |
| `DORIS_URL` | The server (required): `https://…`, or `http://` only for localhost. Only scheme, host and optional port: a path is not supported. |
| `DORIS_COMPANY` | Organisation number or id (optional). Default: the token's only company; with several, the command fails with `company_ambiguous` (exit 2) and lists them. |

## Reference
Global flags, exit codes, error codes and the JSON form of every command are
in [`skills/doris-bookkeeping/reference.md`](../../skills/doris-bookkeeping/reference.md),
which ships with the agent skill.

## For AI agents
The skill `doris-bookkeeping` (`skills/doris-bookkeeping/`) teaches an agent to
keep the books through doris-cli: rehearse with `--dry-run`, read failures by
exit code, never retry a write blindly, moms and common BAS accounts. Install
it in one of three ways:

```
npx skills add hartwigcarlsson/doris --skill doris-bookkeeping   # 40+ agents (skills.sh)
doris-cli skill install [DIR]       # from the binary; default ~/.agents/skills
doris-cli skill show                # print it, e.g. for a system prompt
```

or copy the folder into your agent's skills directory. The skill is built into
the binary, so `doris-cli skill` always matches the CLI's version. Claude Code
finds it in this repository through `.claude/skills/doris-bookkeeping`.
