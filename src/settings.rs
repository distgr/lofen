/* --------------------------
Settings tab
    - Shows music paths configured by the user
    - Allows adding and removing paths
    - Triggers a library scan
-------------------------- */
use crate::tui::App;
use ratatui::{
    prelude::*,
    widgets::*,
    Frame,
};

impl App {
    pub fn render_settings(&self, area: Rect, frame: &mut Frame) {
        let theme = &self.theme;

        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(self.border_type)
            .border_style(theme.resolve(&theme.border_focused))
            .title(" Settings ")
            .title_style(Style::default().fg(theme.resolve(&theme.tab_active_foreground)).bold());

        let inner = block.inner(area);
        frame.render_widget(block, area);

        let layout = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(inner);

        // Header
        let header = Paragraph::new(vec![
            Line::from(""),
            Line::from(vec![
                Span::styled(" Music Paths", Style::default().fg(theme.resolve(&theme.foreground)).bold()),
            ]),
        ]);
        frame.render_widget(header, layout[0]);

        // Music paths list
        let paths = crate::config::get_music_paths(&self.config);

        let path_items: Vec<ListItem> = if paths.is_empty() {
            vec![ListItem::new(Line::from(vec![
                Span::styled(
                    "  No music paths configured.",
                    Style::default().fg(theme.resolve(&theme.foreground_dim)),
                ),
            ]))]
        } else {
            paths
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let selected = self.settings_selected_path == Some(i);
                    let style = if selected {
                        Style::default()
                            .fg(theme.resolve(&theme.foreground))
                            .bg(theme.resolve(&theme.selected_active_background))
                    } else {
                        Style::default().fg(theme.resolve(&theme.foreground))
                    };
                    ListItem::new(Line::from(vec![
                        Span::styled(format!("  {} ", if selected { "▶" } else { " " }), style),
                        Span::styled(p.clone(), style),
                    ]))
                })
                .collect()
        };

        let paths_block = Block::default()
            .borders(Borders::ALL)
            .border_type(self.border_type)
            .border_style(theme.resolve(&theme.border))
            .title(" Paths ");

        frame.render_widget(List::new(path_items).block(paths_block), layout[1]);

        // Footer / keybindings hint
        let hint = if self.settings_adding_path {
            Line::from(vec![
                Span::styled(" Type path and press ", Style::default().fg(theme.resolve(&theme.foreground_dim))),
                Span::styled("Enter", Style::default().fg(theme.primary_color)),
                Span::styled(" to add, ", Style::default().fg(theme.resolve(&theme.foreground_dim))),
                Span::styled("Esc", Style::default().fg(theme.primary_color)),
                Span::styled(" to cancel", Style::default().fg(theme.resolve(&theme.foreground_dim))),
            ])
        } else if let Some(status) = &self.scan_status {
            Line::from(vec![
                Span::styled(" ", Style::default()),
                Span::styled(status.clone(), Style::default().fg(theme.primary_color)),
            ])
        } else {
            Line::from(vec![
                Span::styled(" a", Style::default().fg(theme.primary_color)),
                Span::styled(": add path  ", Style::default().fg(theme.resolve(&theme.foreground_dim))),
                Span::styled("d", Style::default().fg(theme.primary_color)),
                Span::styled(": delete selected  ", Style::default().fg(theme.resolve(&theme.foreground_dim))),
                Span::styled("s", Style::default().fg(theme.primary_color)),
                Span::styled(": scan library", Style::default().fg(theme.resolve(&theme.foreground_dim))),
            ])
        };

        let footer_block = Block::default()
            .borders(Borders::ALL)
            .border_type(self.border_type)
            .border_style(theme.resolve(&theme.border));

        let footer_content = if self.settings_adding_path {
            Paragraph::new(Line::from(vec![
                Span::styled(" Path: ", Style::default().fg(theme.resolve(&theme.foreground_dim))),
                Span::styled(
                    self.settings_path_input.clone() + "█",
                    Style::default().fg(theme.resolve(&theme.foreground)),
                ),
            ]))
            .block(footer_block)
        } else {
            Paragraph::new(hint).block(footer_block)
        };

        frame.render_widget(footer_content, layout[2]);
    }
}
