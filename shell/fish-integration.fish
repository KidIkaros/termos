# TermOS shell integration for Fish
# Add to ~/.config/fish/conf.d/ or source in config.fish:
#   source /path/to/shell/fish-integration.fish
#
# Provides:
#   - OSC 133 semantic prompt markers (A/B/C/D) for command tracking
#   - OSC 7 CWD reporting (current directory)
#   - OSC 0/2 terminal title updates
#   - OSC 52 clipboard: termos_copy / termos_paste / copy / paste

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

    # --- OSC 52 clipboard integration ---

    function termos_copy --description 'Copy text to clipboard via OSC 52'
        set -l text
        if test (count $argv) -gt 0
            set text $argv[1]
        else
            set text (cat)
        end
        set -l encoded (printf '%s' $text | base64 | tr -d '\n')
        printf '\033]52;c;%s\007' $encoded
    end

    function termos_paste --description 'Paste from clipboard'
        if command -q pbpaste
            pbpaste
        else if command -q xclip
            xclip -selection clipboard -o
        else if command -q xsel
            xsel --clipboard --output
        else if command -q wl-paste
            wl-paste
        else
            printf '\033]52;c;?\007' >&2
        end
    end

    function copy --description 'Copy to clipboard (alias for termos_copy)'
        termos_copy $argv
    end

    function paste --description 'Paste from clipboard (alias for termos_paste)'
        termos_paste $argv
    end

    # Set initial title
    printf '\033]0;TermOS — %s\007' (basename $PWD)
end
