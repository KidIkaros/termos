#!/usr/bin/env bash
# TermOS shell integration for Bash
# Source this in your .bashrc: [[ -f /path/to/shell/bash-integration.bash ]] && source /path/to/shell/bash-integration.bash
#
# Provides:
#   - OSC 133 semantic prompt markers (A/B/C/D) for command tracking
#   - OSC 7 CWD reporting (current directory)
#   - OSC 0/2 terminal title updates

if [[ -z "$TERMOS_SHELL_INTEGRATION" ]]; then
    export TERMOS_SHELL_INTEGRATION=1

    # --- OSC 133 semantic markers ---

    # Prompt start marker (A) — emitted before each prompt
    __termos_prompt_start() {
        printf '\033]133;A\007'
    }

    # Command start marker (B) — emitted when user presses Enter
    __termos_command_start() {
        printf '\033]133;B\007'
    }

    # Command executed marker (C) — emitted before command output
    __termos_command_executed() {
        printf '\033]133;C\007'
    }

    # Command finished marker (D) — emitted after command completes
    __termos_command_finished() {
        local exit_code=$?
        printf '\033]133;D;%s\007' "$exit_code"
        return $exit_code
    }

    # --- OSC 7 CWD reporting ---

    __termos_report_cwd() {
        printf '\033]7;file://%s%s\007' "${HOSTNAME:-localhost}" "$PWD"
    }

    # --- OSC 0/2 title updates ---

    __termos_set_title() {
        printf '\033]0;%s\007' "$1"
    }

    # --- Hook into bash preexec/precmd ---

    # preexec: fires before each command (B marker)
    __termos_preexec() {
        __termos_command_start
    }

    # precmd: fires after each command finishes, before prompt (A + D markers)
    __termos_precmd() {
        __termos_command_finished
        __termos_report_cwd
        __termos_prompt_start
    }

    # Use PROMPT_COMMAND for precmd (bash 5.1+ supports arrays)
    if [[ "${BASH_VERSINFO[0]:-0}" -ge 5 && "${BASH_VERSINFO[1]:-0}" -ge 1 ]]; then
        PROMPT_COMMAND+=("__termos_precmd")
    else
        PROMPT_COMMAND="__termos_precmd"
    fi

    # Use DEBUG trap for preexec (fires before each command)
    trap '__termos_command_start' DEBUG

    # Set initial title
    __termos_set_title "TermOS — ${PWD##*/}"
fi
