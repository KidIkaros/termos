# TermOS shell integration for Fish
# Add to ~/.config/fish/conf.d/ or source in config.fish:
#   source /path/to/shell/fish-integration.fish
#
# Provides:
#   - OSC 133 semantic prompt markers (A/B/C/D) for command tracking
#   - OSC 7 CWD reporting (current directory)
#   - OSC 0/2 terminal title updates

if not set -q TERMOS_SHELL_INTEGRATION
    set -gx TERMOS_SHELL_INTEGRATION 1

    # --- Fish event handlers ---

    # fish_prompt: emitted after each command, before the prompt
    function __termos_postexec --on-event fish_postexec
        # D marker: command finished (exit code from $status)
        printf '\033]133;D;%s\007' $status
    end

    function __termos_preexec --on-event fish_preexec
        # B marker: command started
        printf '\033]133;B\007'
    end

    function __termos_prompt --on-event fish_prompt
        # Report CWD
        printf '\033]7;file://%s%s\007' (hostname) $PWD
        # A marker: prompt start
        printf '\033]133;A\007'
    end

    function __termos_title --on-event fish_title
        # Set terminal title
        if test (count $argv) -gt 0
            printf '\033]0;%s\007' "$argv[1]"
        else
            printf '\033]0;TermOS — %s\007' (basename $PWD)
        end
    end

    # Set initial title
    printf '\033]0;TermOS — %s\007' (basename $PWD)
end
