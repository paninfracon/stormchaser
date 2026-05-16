use ratatui::widgets::ListState;
use ratatui_textarea::TextArea;
use stormchaser_tui::app::schema_dialog::SchemaField;

fn main() {
    let mut field = SchemaField {
        name: "role".to_string(),
        description: "".to_string(),
        required: true,
        input: TextArea::default(),
        options: vec![
            "admin".to_string(),
            "developer".to_string(),
            "viewer".to_string(),
        ],
        error: None,
        is_enum: true,
        schema_type: "string".to_string(),
        list_state: ListState::default(),
    };

    // First down press
    let mut i = match field.list_state.selected() {
        Some(i) => {
            if i >= field.options.len() - 1 {
                0
            } else {
                i + 1
            }
        }
        None => 0,
    };
    field.list_state.select(Some(i));
    let mut selected = field.options[i].clone();
    field.input.delete_line_by_head();
    field.input.insert_str(&selected);

    println!("After down 1: {:?}", field.input.lines());

    // Second down press
    i = match field.list_state.selected() {
        Some(i) => {
            if i >= field.options.len() - 1 {
                0
            } else {
                i + 1
            }
        }
        None => 0,
    };
    field.list_state.select(Some(i));
    selected = field.options[i].clone();
    field.input.delete_line_by_head();
    field.input.insert_str(&selected);

    println!("After down 2: {:?}", field.input.lines());

    let text = field.input.lines().join("\n");
    println!("Extracted: '{}'", text);
}
