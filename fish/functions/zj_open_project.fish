# Set up a Zellij tab for coding.
function zj_open_project
    # Open the last diagnostic recorded by bacon, when one exists.
    set -l bacon_location
    if test -f .bacon-locations
        set bacon_location (head -n 1 .bacon-locations)
    end
    set -l project_name (basename "$PWD")

    # Replace the project picker with Helix without passing an empty filename.
    if test -n "$bacon_location"
        zellij run --name "$project_name" --in-place --close-on-exit -- env YAZI=false hx "$bacon_location"
    else
        zellij run --name "$project_name" --in-place --close-on-exit -- env YAZI=false hx
    end

    zellij action rename-tab "$project_name"
    zellij action new-pane -n todo --direction right -- hx -c ~/.config/helix/todo-config.toml TODO.md
    zellij action move-pane left
    zellij action new-pane --direction down --close-on-exit -- fish
    zellij action move-focus right

    zellij action resize increase left
    zellij action resize increase left
    zellij action resize increase left
    zellij action resize increase left

    zellij action new-pane -f -- omp
end
