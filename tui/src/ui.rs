use crate::app::{App, EditField, Mode, Screen, TextInput};
use ratatui::{
    layout::{Constraint, Layout, Position, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use wanna_core::{Quadrant, Want};

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;

/// サイドバーを出す最小の画面幅
const SIDEBAR_MIN_WIDTH: u16 = 100;

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let [body, status] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(area);

    let (main, side) = if area.width >= SIDEBAR_MIN_WIDTH {
        let [m, s] =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(34)]).areas(body);
        (m, Some(s))
    } else {
        (body, None)
    };

    match app.screen {
        Screen::Wants => draw_grid(f, app, main),
        Screen::Done => draw_done(f, app, main),
    }
    if let Some(side) = side {
        draw_sidebar(f, app, side);
    }
    draw_status(f, app, status);
    draw_popup(f, app);
}

fn draw_grid(f: &mut Frame, app: &mut App, area: Rect) {
    let [top_label, grid, bottom_label] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area);

    f.render_widget(Paragraph::new(" エネルギー高".fg(DIM)), top_label);
    f.render_widget(
        Paragraph::new(Line::from(vec![
            " エネルギー低".fg(DIM),
            "      clau低 → clau高".fg(DIM),
        ])),
        bottom_label,
    );

    let rows = Layout::vertical([Constraint::Ratio(1, 2); 2]).split(grid);
    for (qi, q) in Quadrant::ALL.iter().enumerate() {
        let cols = Layout::horizontal([Constraint::Ratio(1, 2); 2]).split(rows[qi / 2]);
        draw_quadrant(f, app, qi, *q, cols[qi % 2]);
    }
}

fn draw_quadrant(f: &mut Frame, app: &mut App, qi: usize, q: Quadrant, area: Rect) {
    let focused = app.cur == qi;
    let list = app.list(q);
    let inner_width = area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = list
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let num = if i < 10 { format!("{:>2} ", (i + 1) % 10) } else { "   ".to_string() };
            let mut spans = vec![Span::styled(num, Style::new().fg(DIM))];
            let mut title = truncate(&w.title, inner_width.saturating_sub(4));
            if !w.notes.is_empty() && title.width() + 2 <= inner_width.saturating_sub(3) {
                title.push_str(" …");
            }
            spans.push(Span::raw(title));
            ListItem::new(Line::from(spans))
        })
        .collect();
    let count = list.len();

    let border = if focused { Style::new().fg(ACCENT) } else { Style::new().fg(DIM) };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if focused { BorderType::Thick } else { BorderType::Rounded })
        .border_style(border)
        .title(Line::from(vec![
            Span::styled(
                format!(" {} ", q.label()),
                if focused { Style::new().fg(ACCENT).bold() } else { Style::new() },
            ),
            Span::styled(format!("{count} "), Style::new().fg(DIM)),
        ]));

    let highlight = if focused {
        Style::new().bg(ACCENT).fg(Color::Black).add_modifier(Modifier::BOLD)
    } else {
        Style::new()
    };
    let widget = List::new(items).block(block).highlight_style(highlight);
    if count == 0 {
        f.render_widget(widget, area);
        let hint = Paragraph::new(" (なし)".fg(DIM));
        let inner = area.inner(ratatui::layout::Margin::new(1, 1));
        f.render_widget(hint, inner);
    } else {
        f.render_stateful_widget(widget, area, &mut app.lists[qi]);
    }
}

fn draw_done(f: &mut Frame, app: &mut App, area: Rect) {
    let done = app.done();
    let title_width = done.iter().map(|w| w.title.width()).max().unwrap_or(0).min(40);
    let items: Vec<ListItem> = done
        .iter()
        .map(|w| {
            let date = w
                .done_at
                .as_deref()
                .and_then(|d| chrono::DateTime::parse_from_rfc3339(d).ok())
                .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| "----------".into());
            let title = pad(&truncate(&w.title, title_width), title_width);
            ListItem::new(Line::from(vec![
                Span::styled(format!("  {date}  "), Style::new().fg(DIM)),
                Span::raw(title),
                Span::styled(format!("  ({})", w.quadrant().label()), Style::new().fg(DIM)),
            ]))
        })
        .collect();
    let count = done.len();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT))
        .title(Line::from(vec![
            Span::styled(" ▼ やったこと ", Style::new().fg(ACCENT).bold()),
            Span::styled(format!("{count} "), Style::new().fg(DIM)),
        ]));
    let widget = List::new(items)
        .block(block)
        .highlight_style(Style::new().bg(ACCENT).fg(Color::Black));
    if count == 0 {
        f.render_widget(widget, area);
        f.render_widget(
            Paragraph::new("  まだありません".fg(DIM)),
            area.inner(ratatui::layout::Margin::new(1, 1)),
        );
    } else {
        f.render_stateful_widget(widget, area, &mut app.done_list);
    }
}

fn draw_sidebar(f: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(DIM));
    let text = match app.selected() {
        Some(w) => detail(w),
        None => vec![Line::from("選択なし".fg(DIM))],
    };
    f.render_widget(Paragraph::new(text).block(block).wrap(Wrap { trim: false }), area);
}

fn detail(w: &Want) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled(w.title.clone(), Style::new().bold())),
        Line::from(Span::styled(w.quadrant().label(), Style::new().fg(ACCENT))),
        Line::from(""),
    ];
    if w.notes.is_empty() {
        lines.push(Line::from("メモなし".fg(DIM)));
    } else {
        lines.extend(w.notes.lines().map(|l| Line::from(l.to_string())));
    }
    lines
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let conn = if !app.has_remote() {
        Span::styled(" ローカルのみ ", Style::new().fg(Color::Black).bg(Color::Gray))
    } else {
        match app.online {
            Some(true) => Span::styled(" 同期 ", Style::new().fg(Color::Black).bg(Color::Green)),
            Some(false) => Span::styled(" オフライン ", Style::new().fg(Color::Black).bg(Color::Yellow)),
            None => Span::styled(" 接続中 ", Style::new().fg(Color::Black).bg(Color::Gray)),
        }
    };
    let mut spans = vec![conn];
    if app.outbox_len > 0 {
        spans.push(Span::styled(format!(" 未送信 {} ", app.outbox_len), Style::new().fg(Color::Yellow)));
    }
    spans.push(Span::raw(" "));
    match &app.message {
        Some(m) => spans.push(Span::styled(m.clone(), Style::new().fg(ACCENT))),
        None => {
            let help = match app.screen {
                Screen::Wants => {
                    "n:追加 e:編集 t:やった d:削除 hjkl:移動 J/K:並べ替え H/L:clau E:エネルギー a:やったこと q:終了"
                }
                Screen::Done => "j/k:移動 u:やったを取り消す a/Esc:戻る q:終了",
            };
            spans.push(Span::styled(help, Style::new().fg(DIM)));
        }
    }
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn popup_area(area: Rect, width: u16, height: u16) -> Rect {
    let w = width.min(area.width.saturating_sub(2));
    let h = height.min(area.height.saturating_sub(2));
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

fn popup_block(title: &str) -> Block<'_> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT))
        .title(Span::styled(format!(" {title} "), Style::new().fg(ACCENT).bold()))
}

/// 2択の表示。選ばれているほうを反転
fn choice(label: &str, a: &str, b: &str, first: bool) -> Line<'static> {
    let on = Style::new().bg(ACCENT).fg(Color::Black).bold();
    let off = Style::new().fg(DIM);
    Line::from(vec![
        Span::raw(format!("{label}  ")),
        Span::styled(format!(" {a} "), if first { on } else { off }),
        Span::raw(" "),
        Span::styled(format!(" {b} "), if first { off } else { on }),
    ])
}

fn draw_popup(f: &mut Frame, app: &App) {
    let area = f.area();
    match &app.mode {
        Mode::Normal => {}
        Mode::AddTitle(input) => {
            let r = popup_area(area, 60, 5);
            f.render_widget(Clear, r);
            let block = popup_block("追加");
            let inner = block.inner(r);
            f.render_widget(block, r);
            let [line, help] =
                Layout::vertical([Constraint::Length(2), Constraint::Length(1)]).areas(inner);
            draw_input(f, "名前 ", input, line, true);
            f.render_widget(Paragraph::new("Enter:次へ  Esc:やめる".fg(DIM)), help);
        }
        Mode::AddEnergy { title, energy } => {
            let r = popup_area(area, 60, 6);
            f.render_widget(Clear, r);
            let text = vec![
                Line::from(title.clone().bold()),
                Line::from(""),
                choice("エネルギー", "高", "低", *energy),
                Line::from("h/l:切替  Enter:次へ  Esc:やめる".fg(DIM)),
            ];
            f.render_widget(Paragraph::new(text).block(popup_block("追加")), r);
        }
        Mode::AddClau { title, energy, clau } => {
            let r = popup_area(area, 60, 7);
            f.render_widget(Clear, r);
            let q = Quadrant { energy: *energy, clau: *clau };
            let text = vec![
                Line::from(title.clone().bold()),
                Line::from(""),
                Line::from(format!("エネルギー  {}", if *energy { "高" } else { "低" }).fg(DIM)),
                choice("clau度    ", "高", "低", *clau),
                Line::from(vec![
                    "h/l:切替  Enter:".fg(DIM),
                    Span::styled(q.label(), Style::new().fg(ACCENT)),
                    " の末尾に追加  Esc:やめる".fg(DIM),
                ]),
            ];
            f.render_widget(Paragraph::new(text).block(popup_block("追加")), r);
        }
        Mode::Edit { title, notes, field, .. } => {
            let r = popup_area(area, 70, 14);
            f.render_widget(Clear, r);
            let block = popup_block("編集");
            let inner = block.inner(r);
            f.render_widget(block, r);
            let [t, n, help] = Layout::vertical([
                Constraint::Length(2),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .areas(inner);
            draw_input(f, "名前 ", title, t, *field == EditField::Title);
            draw_input(f, "メモ ", notes, n, *field == EditField::Notes);
            let h = match field {
                EditField::Title => "Enter/Ctrl+S:保存  Tab:メモへ  Esc:やめる",
                EditField::Notes => "Ctrl+S:保存  Enter:改行  Tab:名前へ  Esc:やめる",
            };
            f.render_widget(Paragraph::new(h.fg(DIM)), help);
        }
        Mode::ConfirmDelete { title, .. } => {
            let r = popup_area(area, 50, 5);
            f.render_widget(Clear, r);
            let text = vec![
                Line::from(truncate(title, 44).bold()),
                Line::from("削除しますか？ (y/n)"),
            ];
            f.render_widget(Paragraph::new(text).block(popup_block("削除")), r);
        }
    }
}

/// ラベル付きの入力欄。複数行 (メモ) にも対応し、focus ならカーソルを置く
fn draw_input(f: &mut Frame, label: &str, input: &TextInput, area: Rect, focus: bool) {
    let label_w = label.width() as u16;
    let style = if focus { Style::new() } else { Style::new().fg(DIM) };
    let mut lines: Vec<Line> = Vec::new();
    for (i, l) in input.text.split('\n').enumerate() {
        let head = if i == 0 { label.to_string() } else { " ".repeat(label_w as usize) };
        lines.push(Line::from(vec![
            Span::styled(head, Style::new().fg(if focus { ACCENT } else { DIM })),
            Span::styled(l.to_string(), style),
        ]));
    }
    // カーソルの行・列。長い行はスクロールさせず、見える範囲に収める
    let before = input.before_cursor();
    let row = before.matches('\n').count() as u16;
    let col = before.rsplit('\n').next().unwrap_or("").width() as u16;
    let scroll = row.saturating_sub(area.height.saturating_sub(1));
    f.render_widget(Paragraph::new(lines).scroll((scroll, 0)), area);
    if focus {
        let x = (area.x + label_w + col).min(area.right().saturating_sub(1));
        f.set_cursor_position(Position::new(x, area.y + row - scroll));
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_string();
    }
    let mut out = String::new();
    let mut w = 0;
    for c in s.chars() {
        let cw = c.width().unwrap_or(0);
        if w + cw + 1 > max {
            break;
        }
        out.push(c);
        w += cw;
    }
    out.push('…');
    out
}

fn pad(s: &str, width: usize) -> String {
    format!("{s}{}", " ".repeat(width.saturating_sub(s.width())))
}
