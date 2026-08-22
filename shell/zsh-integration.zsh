#!/usr/bin/env zsh
# TermOS shell integration for Zsh
# Source this in your .zshrc: [[ -f /path/to/shell/zsh-integration.zsh ]] && source /path/to/shell/zsh-integration.zsh
#
# Provides:
#   - OSC 133 semantic prompt markers (A/B/C/D) for command tracking
#   - OSC 7 CWD reporting (current directory)
#   - OSC 0/2 terminal title updates

if [[ -z "$TERMOS_SHELL_INTEGRATION" ]]; then
    export TERMOS_SHELL_INTEGRATION=1

    # --- OSC 133 semantic markers ---

    __termos_prompt_start() {
        printf '\033]133;A\007'
    }

    __termos_command_start() {
        printf '\033]133;B\007'
    }

    __termos_command_executed() {
        printf '\033]133;C\007'
    }

    __termos_command_finished() {
        printf '\033]133;D;%s\007' "$?"
    }

    # --- OSC 7 CWD reporting ---

    __termos_report_cwd() {
        printf '\033]7;file://%s%s\007' "${HOSTNAME:-localhost}" "$PWD"
    }

    # --- OSC 0/2 title updates ---

    __termos_set_title() {
        printf '\033]0;%s\007' "$1"
    }

    # --- Zsh hooks ---

    # precmd: after each command, before prompt
    __termos_precmd() {
        __termos_command_finished
        __termos_report_cwd
        __termos_prompt_start
    }

    # preexec: before each command runs
    __termos_preexec() {
        __termos_command_start
    }

    # chpwd: when directory changes
    __termos_chpwd() {
        __termos_report_cwd
    }

    autoload -Uz add-zsh-hook
    add-zsh-hook precmd __termos_precmd
    add-zsh-hook preexec __termos_preexec
    add-zsh-hook chpwd __termos_chpwd

    # Set initial title
    __termos_set_title "TermOS — ${PWD##*/}"
fi
