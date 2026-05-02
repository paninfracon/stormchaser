import os
import re

with open('crates/stormchaser-tui/src/ui.rs', 'r') as f:
    lines = f.readlines()

blocks = []
current_block = []
current_name = 'imports'

for line in lines:
    m = re.match(r'^(pub )?fn ([a-zA-Z0-9_]+)', line)
    if m:
        blocks.append((current_name, current_block))
        current_block = [line]
        current_name = m.group(2)
    else:
        current_block.append(line)
blocks.append((current_name, current_block))

os.makedirs('crates/stormchaser-tui/src/ui', exist_ok=True)

modules = {
    'utils': ['centered_rect', 'format_status', 'format_time_str'],
    'runs': ['render_runs_tab', 'render_run_detail', 'render_test_results'],
    'storage': ['render_storage_backends_tab', 'render_storage_backend_dialog'],
    'webhooks': ['render_webhooks_tab', 'render_webhook_dialog'],
    'dialogs': ['render_filter_dialog', 'render_file_browser', 'render_schedule_git_dialog'],
    'mod': ['imports', 'ui', 'render_login_screen']
}

for mod_name, funcs in modules.items():
    with open(f'crates/stormchaser-tui/src/ui/{mod_name}.rs', 'w') as f:
        for b_name, b_lines in blocks:
            if b_name in funcs:
                f.write(''.join(b_lines))
