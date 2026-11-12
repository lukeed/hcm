# hcm

> Move your Herdr TUI workspaces into herdr-core (`hc`).

`hcm` rebuilds the workspaces from your Herdr TUI session inside `hc`, so they show up in [rodeo](https://github.com/lukeed/rodeo) & the Herdr app. Same workspaces, same tabs (& tab names), same folders.

It can't move running programs though. Every terminal in Herdr belongs to the `herdr` server, and `hc` can't adopt them. So each tab gets a fresh shell in its folder instead. Claude & Codex panes can pick their conversation back up afterward (see [step 3](#3-resume-your-agents)).

I wrote it to move my own 17 workspaces off Herdr TUI, so it's a one-time tool, not a sync.

## What moves

* **Moves:** workspaces & their custom names, tabs & their custom names, each tab's folder, and each Claude or Codex pane's conversation
* **Doesn't move:** running programs (dev servers, watchers, …), scrollback, anything unsaved inside a program
* **Not supported:** tabs with split panes; `hcm` stops & says which one

## Requirements

* Herdr TUI 0.9.5, which writes session snapshots to `~/.config/herdr/session-snapshots/`
* `hc` 0.1.18 or newer, on your `PATH`

## Install

```sh
$ mise use -g packslip:github.com/lukeed/hcm
# or
$ packslip install github.com/lukeed/hcm
```

Or grab the archive for your platform from the [latest release](https://github.com/lukeed/hcm/releases/latest). It holds the `hcm` binary alone.

## Usage

There are three steps, and you can stop after any of them.

### 1. Look first

```sh
$ hcm plan
```

This prints every workspace & tab that `hcm` would create, with each agent's resume command. It changes nothing.

### 2. Move the layout

```sh
$ hcm apply
```

For every tab, this starts a shell in `hc`, in that tab's folder. Then it adds the spaces & tabs to the layout that `hc` shares with rodeo & the Herdr app.

* **It merges.** A folder that already has a space in `hc` gets the new tabs added to it. Every other workspace becomes a new space. Nothing you already have is changed.
* **It's safe to run again.** It skips any tab it already moved.

Herdr keeps running the whole time, so open `rodeo` and compare the two side by side.

### 3. Resume your agents

When you're happy, stop Herdr. Run this from a terminal that is **not** inside Herdr, because it ends every program there:

```sh
$ herdr server stop
```

Then, from a terminal in rodeo:

```sh
$ hcm resume
```

This types `claude --resume <id>` (or `codex resume <id>`) into each agent's new pane, once.

> **Why wait for Herdr to stop?** Until then, the Herdr copy of each agent is still running. Resume it in `hc` too, and the same conversation runs twice.

## How `resume` knows what to resume

You never tell it. `apply` already saved that in `hc`:

1. Herdr's snapshot records each pane's agent & its conversation ID.
2. For each pane with one, `apply` labels its new `hc` terminal with the resume command (`migrate.resume`).
3. `resume` reads those labels back from `hc`, types each command, then empties the label.

So `resume` doesn't need Herdr at all, and running it twice does nothing the second time.

One catch: the IDs are from when you ran `apply`. If an agent started a new conversation after that (say, after a `/clear`), `resume` picks up the old one. For that pane, run `claude --resume` yourself and choose the right conversation.

## Options

* `--snapshot <file>`: use this snapshot, not the newest one
* `HC_SESSION=<name>`: move into another `hc` session, the same as for `hc` itself

## Development

```sh
$ mise install
$ mise run check   # fmt, clippy, jactionlint & tests
```

The tests start their own isolated `hc` core, so they never touch your terminals.

## License

MIT © Luke Edwards
