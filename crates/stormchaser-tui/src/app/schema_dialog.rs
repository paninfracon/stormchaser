use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph};
use ratatui::Frame;
use ratatui_textarea::TextArea;
use serde_json::Value;

pub struct SchemaField<'a> {
    pub name: String,
    pub description: String,
    pub required: bool,
    pub input: TextArea<'a>,
    pub options: Vec<String>,
    pub error: Option<String>,
    pub is_enum: bool,
    pub schema_type: String,
    pub list_state: ListState,
}

pub struct SchemaDialog<'a> {
    pub fields: Vec<SchemaField<'a>>,
    pub focus: usize,
    pub scroll_offset: usize,
    pub base_schema: Value,
    pub dsl: String,
    pub is_hydrating: bool,
    pub hydration_status: String,
    pub global_errors: Vec<String>,
}

impl<'a> SchemaDialog<'a> {
    pub fn new(schema: Value, dsl: String, inputs: Value) -> Self {
        let mut fields = Vec::new();

        let flat_schema = stormchaser_model::schema_gen::flatten_schema_for_ui(&schema, &inputs);

        let required_fields: Vec<String> = flat_schema
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        if let Some(properties) = flat_schema.get("properties").and_then(|v| v.as_object()) {
            for (key, prop) in properties {
                let description = prop
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let mut input = TextArea::default();
                input.set_cursor_line_style(Style::default());
                input.set_block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" {} ", key)),
                );

                if let Some(val) = inputs.get(key) {
                    if let Some(s) = val.as_str() {
                        input.insert_str(s);
                    } else {
                        input.insert_str(val.to_string());
                    }
                } else if let Some(def) = prop.get("default") {
                    if let Some(s) = def.as_str() {
                        input.insert_str(s);
                    } else {
                        input.insert_str(def.to_string());
                    }
                }

                let mut options = Vec::new();
                let mut is_enum = false;
                if let Some(enum_vals) = prop.get("enum").and_then(|v| v.as_array()) {
                    is_enum = true;
                    for v in enum_vals {
                        if let Some(s) = v.as_str() {
                            options.push(s.to_string());
                        } else {
                            options.push(v.to_string());
                        }
                    }
                }

                fields.push(SchemaField {
                    name: key.clone(),
                    description,
                    required: required_fields.contains(key),
                    input,
                    options,
                    error: None,
                    is_enum,
                    schema_type: prop
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("string")
                        .to_string(),
                    list_state: ListState::default(),
                });
            }
        }

        Self {
            fields,
            focus: 0,
            scroll_offset: 0,
            base_schema: schema,
            dsl,
            is_hydrating: false,
            hydration_status: "Ready".to_string(),
            global_errors: Vec::new(),
        }
    }

    pub fn get_inputs(&self) -> Value {
        let mut map = serde_json::Map::new();
        for field in &self.fields {
            let text = field.input.lines().join("\n");
            if !text.is_empty() {
                let parsed_val = match field.schema_type.as_str() {
                    "integer" | "number" => text
                        .parse::<i64>()
                        .map(|n| Value::Number(n.into()))
                        .unwrap_or(Value::String(text)),
                    "boolean" => text
                        .parse::<bool>()
                        .map(Value::Bool)
                        .unwrap_or(Value::String(text)),
                    _ => Value::String(text),
                };
                map.insert(field.name.clone(), parsed_val);
            }
        }
        Value::Object(map)
    }

    pub fn apply_hydrated_schema(&mut self, schema: Value, errors: Vec<String>, status: String) {
        self.hydration_status = status;
        self.global_errors = errors;

        let required_fields: Vec<String> = schema
            .get("required")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        let valid_keys: std::collections::HashSet<String> =
            if let Some(properties) = schema.get("properties").and_then(|v| v.as_object()) {
                properties.keys().cloned().collect()
            } else {
                std::collections::HashSet::new()
            };

        self.fields.retain(|field| valid_keys.contains(&field.name));

        if self.focus >= self.fields.len() && !self.fields.is_empty() {
            self.focus = self.fields.len() - 1;
        } else if self.fields.is_empty() {
            self.focus = 0;
        }

        if self.scroll_offset > self.focus {
            self.scroll_offset = self.focus;
        }

        if let Some(properties) = schema.get("properties").and_then(|v| v.as_object()) {
            let mut new_fields = Vec::with_capacity(properties.len());
            for (key, prop) in properties {
                let mut existing_field = None;
                for (i, field) in self.fields.iter().enumerate() {
                    if field.name == *key {
                        existing_field = Some(self.fields.remove(i));
                        break;
                    }
                }

                if let Some(mut field) = existing_field {
                    field.required = required_fields.contains(key);
                    if let Some(enum_vals) = prop.get("enum").and_then(|v| v.as_array()) {
                        field.is_enum = true;
                        field.options.clear();
                        for v in enum_vals {
                            if let Some(s) = v.as_str() {
                                field.options.push(s.to_string());
                            } else {
                                field.options.push(v.to_string());
                            }
                        }
                    }
                    new_fields.push(field);
                } else {
                    let description = prop
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let mut input = TextArea::default();
                    input.set_cursor_line_style(Style::default());
                    input.set_block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title(format!(" {} ", key)),
                    );

                    if let Some(def) = prop.get("default") {
                        if let Some(s) = def.as_str() {
                            input.insert_str(s);
                        } else {
                            input.insert_str(def.to_string());
                        }
                    }

                    let mut options = Vec::new();
                    let mut is_enum = false;
                    if let Some(enum_vals) = prop.get("enum").and_then(|v| v.as_array()) {
                        is_enum = true;
                        for v in enum_vals {
                            if let Some(s) = v.as_str() {
                                options.push(s.to_string());
                            } else {
                                options.push(v.to_string());
                            }
                        }
                    }

                    new_fields.push(SchemaField {
                        name: key.clone(),
                        description,
                        required: required_fields.contains(key),
                        input,
                        options,
                        error: None,
                        is_enum,
                        schema_type: prop
                            .get("type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("string")
                            .to_string(),
                        list_state: ratatui::widgets::ListState::default(),
                    });
                }
            }
            self.fields = new_fields;
        }
        self.is_hydrating = false;
    }

    pub fn next_field(&mut self) {
        if !self.fields.is_empty() {
            self.focus = (self.focus + 1) % self.fields.len();
        }
    }

    pub fn prev_field(&mut self) {
        if !self.fields.is_empty() {
            if self.focus == 0 {
                self.focus = self.fields.len() - 1;
            } else {
                self.focus -= 1;
            }
        }
    }

    pub fn handle_dropdown_down(&mut self) {
        if let Some(field) = self.fields.get_mut(self.focus) {
            if field.is_enum && !field.options.is_empty() {
                let i = match field.list_state.selected() {
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
                let selected = field.options[i].clone();
                field.input.delete_line_by_head();
                field.input.insert_str(selected);
            }
        }
    }

    pub fn handle_dropdown_up(&mut self) {
        if let Some(field) = self.fields.get_mut(self.focus) {
            if field.is_enum && !field.options.is_empty() {
                let i = match field.list_state.selected() {
                    Some(i) => {
                        if i == 0 {
                            field.options.len() - 1
                        } else {
                            i - 1
                        }
                    }
                    None => field.options.len() - 1,
                };
                field.list_state.select(Some(i));
                let selected = field.options[i].clone();
                field.input.delete_line_by_head();
                field.input.insert_str(selected);
            }
        }
    }
}

pub fn draw_schema_dialog(f: &mut Frame, dialog: &mut SchemaDialog) {
    let size = f.area();
    let width = std::cmp::min(60, size.width.saturating_sub(4));
    let height = std::cmp::min(size.height.saturating_sub(4), 24);

    let x = (size.width.saturating_sub(width)) / 2;
    let y = (size.height.saturating_sub(height)) / 2;
    let area = Rect::new(x, y, width, height);

    f.render_widget(Clear, area);

    let title = format!(" Workflow Inputs - [{}] ", dialog.hydration_status);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(Style::default().fg(Color::Cyan));
    f.render_widget(block, area);

    let inner_area = Rect::new(area.x + 2, area.y + 1, area.width - 4, area.height - 2);

    let visible_fields_count = (inner_area.height.saturating_sub(2) / 3) as usize; // error + footer
    let total_fields = dialog.fields.len();
    let visible_fields_count = std::cmp::max(1, visible_fields_count);

    if dialog.focus < dialog.scroll_offset {
        dialog.scroll_offset = dialog.focus;
    } else if dialog.focus >= dialog.scroll_offset + visible_fields_count {
        dialog.scroll_offset = dialog.focus.saturating_sub(visible_fields_count - 1);
    }

    let end_idx = std::cmp::min(total_fields, dialog.scroll_offset + visible_fields_count);
    let start_idx = dialog.scroll_offset;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length((end_idx.saturating_sub(start_idx)) as u16 * 3), // Fields
            Constraint::Min(0),                                                 // Spacer
            Constraint::Length(1),                                              // Errors
            Constraint::Length(1),                                              // Footer
        ])
        .split(inner_area);

    let mut field_y = chunks[0].y;
    let mut dropdown_area = None;

    for i in start_idx..end_idx {
        let field = &mut dialog.fields[i];
        let field_rect = Rect::new(chunks[0].x, field_y, chunks[0].width, 3);
        field_y += 3;

        let style = if i == dialog.focus {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        let title = if field.required {
            format!(" {}* ", field.name)
        } else {
            format!(" {} ", field.name)
        };

        let mut b = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .border_style(style);

        if i == start_idx && start_idx > 0 {
            b = b.title_top("↑ More");
        }
        if i == end_idx - 1 && end_idx < total_fields {
            b = b.title_bottom("↓ More");
        }

        field.input.set_block(b);
        f.render_widget(&field.input, field_rect);

        if i == dialog.focus && field.is_enum {
            dropdown_area = Some((field_rect.x, field_rect.y + 3, field_rect.width));
        }
    }

    if let Some((dx, dy, dwidth)) = dropdown_area {
        let field = &mut dialog.fields[dialog.focus];
        if !field.options.is_empty() {
            let max_list_height = inner_area.bottom().saturating_sub(dy).saturating_sub(2);
            let list_height = std::cmp::min(field.options.len() as u16, max_list_height);
            if list_height > 0 {
                let drop_area = Rect::new(dx, dy, dwidth, list_height + 2);
                let items: Vec<ListItem> = field
                    .options
                    .iter()
                    .map(|o| ListItem::new(o.clone()))
                    .collect();
                let list = List::new(items)
                    .block(Block::default().borders(Borders::ALL))
                    .highlight_style(Style::default().bg(Color::Blue).fg(Color::White))
                    .highlight_symbol(">> ");
                f.render_widget(Clear, drop_area); // Overwrite underlying fields
                f.render_stateful_widget(list, drop_area, &mut field.list_state);
            }
        }
    }

    let err_msg = dialog.global_errors.join(", ");
    if !err_msg.is_empty() {
        let err_para = Paragraph::new(err_msg).style(Style::default().fg(Color::Red));
        f.render_widget(err_para, chunks[2]);
    }

    let footer = Paragraph::new(
        "Tab: Next | Shift+Tab: Prev | Up/Down: Options | Enter: Submit | Esc: Cancel",
    )
    .style(Style::default().fg(Color::DarkGray));
    f.render_widget(footer, chunks[3]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_apply_hydrated_schema_adds_new_fields() {
        let base_schema = json!({
            "type": "object",
            "properties": {
                "type": { "type": "string" }
            }
        });

        let mut dialog = SchemaDialog::new(base_schema, "".to_string(), json!({}));
        assert_eq!(dialog.fields.len(), 1);
        assert_eq!(dialog.fields[0].name, "type");

        let hydrated_schema = json!({
            "type": "object",
            "properties": {
                "type": { "type": "string" },
                "spec": { "type": "object", "description": "The spec" }
            },
            "required": ["spec"]
        });

        dialog.apply_hydrated_schema(hydrated_schema, vec![], "Completed".to_string());

        assert_eq!(dialog.fields.len(), 2);
        assert_eq!(dialog.fields[0].name, "type");
        assert_eq!(dialog.fields[1].name, "spec");
        assert_eq!(dialog.fields[1].description, "The spec");
        assert!(dialog.fields[1].required);
    }
}
