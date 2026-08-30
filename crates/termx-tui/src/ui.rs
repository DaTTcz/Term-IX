use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph};
use ratatui::Frame;

use crate::app::{App, Field, Mode};

pub fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(3), Constraint::Min(3), Constraint::Length(3)])
        .split(f.area());

    draw_title(f, chunks[0]);
    draw_session_list(f, chunks[1], app);
    draw_help(f, chunks[2]);

    match &app.mode {
        Mode::AddForm { field, values } => draw_add_form(f, *field, values),
        Mode::ConfirmDelete => draw_confirm_delete(f),
        Mode::Message(msg) => draw_message(f, msg),
        Mode::Normal => {}
    }
}

fn draw_title(f: &mut Frame, area: Rect) {
    let title = Paragraph::new("Term-IX")
        .alignment(Alignment::Center)
        .style(Style::default().add_modifier(Modifier::BOLD).fg(Color::Cyan))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(title, area);
}

fn draw_session_list(f: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app
        .sessions()
        .iter()
        .map(|s| {
            let line = Line::from(vec![
                Span::styled(format!("{:<22}", s.name), Style::default().add_modifier(Modifier::BOLD)),
                Span::raw(format!(" [{}]  {}:{}", s.protocol, s.host, s.port)),
            ]);
            ListItem::new(line)
        })
        .collect();

    let title = if items.is_empty() {
        "Servery (prazdno - stisknete 'a' pro pridani)".to_string()
    } else {
        format!("Servery ({})", items.len())
    };

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(Style::default().bg(Color::Blue).add_modifier(Modifier::BOLD))
        .highlight_symbol("> ");

    let mut state = app.list_state.clone();
    f.render_stateful_widget(list, area, &mut state);
}

fn draw_help(f: &mut Frame, area: Rect) {
    let help = Paragraph::new(
        "Enter: pripojit  |  a: pridat  |  d: smazat  |  \u{2191}/\u{2193}: vyber  |  q/Esc: konec",
    )
    .alignment(Alignment::Center)
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(help, area);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn draw_add_form(f: &mut Frame, field: usize, values: &[String; 5]) {
    let area = centered_rect(60, 60, f.area());
    f.render_widget(Clear, area);

    let mut lines = vec![Line::from("Novy server (SSH) - Tab/Enter dalsi pole, Esc zrusit"), Line::from("")];

    for (i, fld) in Field::ALL.iter().enumerate() {
        let marker = if i == field { "> " } else { "  " };
        let masked = if matches!(fld, Field::Password) {
            "*".repeat(values[i].chars().count())
        } else {
            values[i].clone()
        };
        let style = if i == field {
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(format!("{marker}{:<10}: {masked}", fld.label()), style)));
    }

    let block = Block::default().borders(Borders::ALL).title("Pridat server");
    let p = Paragraph::new(lines).block(block);
    f.render_widget(p, area);
}

fn draw_confirm_delete(f: &mut Frame) {
    let area = centered_rect(40, 20, f.area());
    f.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).title("Smazat server?");
    let p = Paragraph::new("Opravdu smazat vybrany server? [y/N]")
        .alignment(Alignment::Center)
        .block(block);
    f.render_widget(p, area);
}

fn draw_message(f: &mut Frame, msg: &str) {
    let area = centered_rect(60, 30, f.area());
    f.render_widget(Clear, area);
    let block = Block::default().borders(Borders::ALL).title("Term-IX");
    let p = Paragraph::new(format!("{msg}\n\n(libovolna klavesa pro zavreni)"))
        .alignment(Alignment::Center)
        .block(block);
    f.render_widget(p, area);
}
