# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

`waffles` is a Rust CLI tool that runs shell commands in parallel, displaying their output with labeled prefixes. Commands can be provided via a script file, stdin (`--stdin`), or repeated `-c` flags. It uses rayon for parallel execution. Exit code equals the number of failed commands (capped at 20).

With `--named`, each line from a script file or `--stdin` must be a named task of the form `name: command`, where `name` matches `[a-z0-9_-]+`. The name is used as the output label (and `{cmd}` filename placeholder) instead of the command text. Malformed lines, empty commands, and duplicate names are rejected up front. The flag does not affect `-c`/`--command` commands.

## Build & Test

Standard Rust/Cargo workflow.

## Dependencies

Do not add new cargo dependencies.

## Architecture

Single-file CLI (`src/main.rs`): argument parsing (clap derive), task parsing, command execution, and output formatting all live in one file. Integration tests in `tests/integration.rs` invoke the compiled binary via `std::process::Command`.

Output format: stdout lines are prefixed with `label | line`, stderr lines with `label ! line`. In quiet mode (`-q`), output is suppressed for successful commands and only shown for failures.

Uses Rust edition 2024.
