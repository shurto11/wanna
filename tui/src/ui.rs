use crate::app::{help_lines, matches, App, Edit, EditField, Mode, Screen, HELP};
use crate::input::TextInput;
use ratatui::{
    layout::{Alignment, Constraint, Layout, Position, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Wrap},
    Frame,
};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use wanna_core::{due, Kind, Quadrant, Want};

const ACCENT: Color = Color::Cyan;
const DIM: Color = Color::DarkGray;
/// 日時が過ぎている / 今日
const OVER: Color = Color::Red;
const TODAY: Color = Color::Yellow;
/// ビジュアルモードで選んでいる行
const VISUAL: Color = Color::Blue;

/// やったことリストの名前 (Kind::label と並ぶ3つめ)
const DONE: &str = "Done";
/// 保管庫の名前 (4つめ)
const ARCHIVE: &str = "Archive";

/// サイドバーを出す最小の画面幅
const SIDEBAR_MIN_WIDTH: u16 = 100;

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let [tabs, body, status] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(0), Constraint::Length(1)])
            .areas(area);

    let (main, side) = if area.width >= SIDEBAR_MIN_WIDTH {
        let [m, s] =
            Layout::horizontal([Constraint::Min(0), Constraint::Length(34)]).areas(body);
        (m, Some(s))
    } else {
        (body, None)
    };

    draw_tabs(f, app, tabs);
    match app.screen {
        Screen::List => draw_grid(f, app, main),
        Screen::Done | Screen::Archive | Screen::Memo => draw_past(f, app, main),
    }
    if let Some(side) = side {
        draw_sidebar(f, app, side);
    }
    draw_status(f, app, status);
    // キー一覧は見えている高さより先へスクロールさせない
    if let Mode::Help(scroll) = &mut app.mode {
        let rows = help_area(area).height.saturating_sub(2) as usize;
        *scroll = (*scroll).min(help_lines().saturating_sub(rows) as u16);
    }
    draw_popup(f, app);
}

/// Want / Must / Done / Archive / Memo の切り替え位置を出す。
/// 囲みの `[` `]` がそのまま切り替えのキー
fn draw_tabs(f: &mut Frame, app: &App, area: Rect) {
    let on = Style::new().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD);
    let off = Style::new().fg(DIM);
    let selected = |k: Kind| app.screen == Screen::List && app.kind == k;
    let mut spans = vec![Span::styled("[", Style::new().fg(DIM))];
    for (name, sel) in [
        (Kind::Want.label(), selected(Kind::Want)),
        (Kind::Task.label(), selected(Kind::Task)),
        (DONE, app.screen == Screen::Done),
        (ARCHIVE, app.screen == Screen::Archive),
        (Kind::Memo.label(), app.screen == Screen::Memo),
    ] {
        spans.push(Span::styled(format!(" {name} "), if sel { on } else { off }));
        spans.push(Span::raw(" "));
    }
    spans.pop();
    spans.push(Span::styled("]", Style::new().fg(DIM)));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// 2×2 の区分を1つの枠に収め、中央に2軸の矢印を引く。
/// 上が縦軸の高、右が clau高 (右上 = 高・高、左下 = 低・低)
fn draw_grid(f: &mut Frame, app: &mut App, area: Rect) {
    let kind = app.kind;
    let frame = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(DIM));
    let inner = frame.inner(area);
    f.render_widget(frame, area);

    let [top, row0, hgap, row1, bottom] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    let cols = |r: Rect| -> [Rect; 3] {
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(1), Constraint::Fill(1)]).areas(r)
    };
    let axis = Style::new().fg(DIM);

    // 縦軸: やりたいことならエネルギー、次にやることなら重要度 (上が高)
    let [_, mid, right] = cols(top);
    f.render_widget(Paragraph::new(Span::styled("▲", axis)), mid);
    f.render_widget(Paragraph::new(Span::styled(format!(" {}高", kind.axis()), axis)), right);
    let [_, mid, right] = cols(bottom);
    f.render_widget(Paragraph::new(Span::styled("│", axis)), mid);
    f.render_widget(Paragraph::new(Span::styled(format!(" {}低", kind.axis()), axis)), right);
    for r in [row0, row1] {
        let [_, mid, _] = cols(r);
        let bar = vec![Line::from("│"); mid.height as usize];
        f.render_widget(Paragraph::new(bar).style(axis), mid);
    }

    // 横軸: clau度 (右が高)
    let [left, mid, right] = cols(hgap);
    let lo = " clau低 ";
    let hi = "▶ clau高 ";
    let left_line = format!("{lo}{}", "─".repeat((left.width as usize).saturating_sub(lo.width())));
    let right_line = format!("{}{hi}", "─".repeat((right.width as usize).saturating_sub(hi.width())));
    f.render_widget(Paragraph::new(Span::styled(left_line, axis)), left);
    f.render_widget(Paragraph::new(Span::styled("┼", axis)), mid);
    f.render_widget(Paragraph::new(Span::styled(right_line, axis)), right);

    for (qi, q) in Quadrant::ALL.iter().enumerate() {
        let [l, _, r] = cols(if qi < 2 { row0 } else { row1 });
        draw_quadrant(f, app, qi, *q, if qi % 2 == 0 { l } else { r });
    }
}

fn draw_quadrant(f: &mut Frame, app: &mut App, qi: usize, q: Quadrant, area: Rect) {
    let kind = app.kind;
    let focused = app.cur == qi;
    let list = app.list(kind, q);
    let now = chrono::Local::now().naive_local();
    // 日時の欄は区分ごとに幅を揃える。日時が1つも無ければ欄そのものを出さない
    let dues: Vec<Option<String>> =
        list.iter().map(|w| w.due().map(|d| due::format(d, now.date()))).collect();
    let due_w = dues.iter().flatten().map(|d| d.width()).max().unwrap_or(0);
    // 行番号 (`5G` で飛べる)。~/.vimrc の `number` と同じく全行に出す
    let num_w = list.len().to_string().len().max(2);
    let inner_width = area.width.saturating_sub(2) as usize;
    let title_width = inner_width.saturating_sub(num_w + 2 + if due_w > 0 { due_w + 1 } else { 0 });
    let visual = if focused { app.visual_range() } else { None };
    let query = app.search.clone();

    let items: Vec<ListItem> = list
        .iter()
        .zip(&dues)
        .enumerate()
        .map(|(i, (w, d))| {
            let in_visual = visual.as_ref().is_some_and(|r| r.contains(&i));
            let num_style = if in_visual { Style::new() } else { Style::new().fg(DIM) };
            let mut spans = vec![Span::styled(format!("{:>num_w$} ", i + 1), num_style)];
            if due_w > 0 {
                let text = pad(d.as_deref().unwrap_or(""), due_w);
                let style = match w.due().map(|s| due::state(s, now)) {
                    Some(due::State::Over) => Style::new().fg(OVER).add_modifier(Modifier::BOLD),
                    Some(due::State::Today) => Style::new().fg(TODAY),
                    _ => Style::new().fg(DIM),
                };
                spans.push(Span::styled(format!("{text} "), style));
            }
            let mut title = truncate(&w.title, title_width);
            if !w.notes.is_empty() && title.width() + 2 <= title_width.saturating_sub(1) {
                title.push_str(" …");
            }
            spans.extend(title_spans(title, query.as_deref()));
            item(spans, in_visual)
        })
        .collect();
    let empty = list.is_empty();
    if focused {
        app.view_rows = area.height.saturating_sub(2) as usize;
    }

    let border = if focused { Style::new().fg(ACCENT) } else { Style::new().fg(DIM) };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if focused { BorderType::Thick } else { BorderType::Rounded })
        .border_style(border);

    let highlight = if focused {
        Style::new().bg(ACCENT).fg(Color::Black).add_modifier(Modifier::BOLD)
    } else {
        Style::new()
    };
    let widget = List::new(items).block(block).highlight_style(highlight);
    if empty {
        f.render_widget(widget, area);
        let hint = Paragraph::new(" (なし)".fg(DIM));
        let inner = area.inner(ratatui::layout::Margin::new(1, 1));
        f.render_widget(hint, inner);
    } else {
        f.render_stateful_widget(widget, area, &mut app.lists[kind.index()][qi]);
    }
}

/// やったこと / 保管庫 / メモ。日付はやった日 / しまった日 / 作った日
fn draw_past(f: &mut Frame, app: &mut App, area: Rect) {
    let screen = app.screen;
    let (list, name) = match screen {
        Screen::Archive => (app.archived(), ARCHIVE),
        Screen::Memo => (app.memos(), Kind::Memo.label()),
        _ => (app.done(), DONE),
    };
    let memo = screen == Screen::Memo;
    let title_width = list.iter().map(|w| w.title.width()).max().unwrap_or(0).min(40);
    let visual = app.visual_range();
    let query = app.search.clone();
    let items: Vec<ListItem> = list
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let in_visual = visual.as_ref().is_some_and(|r| r.contains(&i));
            let date = match screen {
                Screen::Archive => w.archived_at.as_deref(),
                Screen::Memo => Some(w.created_at.as_str()),
                _ => w.done_at.as_deref(),
            };
            let date = date.map(local_date).unwrap_or_else(|| "----------".into());
            let dim = if in_visual { Style::new() } else { Style::new().fg(DIM) };
            let mut spans = vec![Span::styled(format!("  {date}  "), dim)];
            if memo {
                spans.extend(title_spans(w.title.clone(), query.as_deref()));
            } else {
                let title = pad(&truncate(&w.title, title_width), title_width);
                spans.extend(title_spans(title, query.as_deref()));
                spans.push(Span::styled(format!("  ({} / {})", w.kind.label(), w.quadrant_label()), dim));
            }
            item(spans, in_visual)
        })
        .collect();
    let empty = list.is_empty();
    app.view_rows = area.height.saturating_sub(2) as usize;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT))
        .title(Span::styled(format!(" ▼ {name} "), Style::new().fg(ACCENT).bold()));
    let widget = List::new(items)
        .block(block)
        .highlight_style(Style::new().bg(ACCENT).fg(Color::Black));
    if empty {
        f.render_widget(widget, area);
        f.render_widget(
            Paragraph::new("  まだありません".fg(DIM)),
            area.inner(ratatui::layout::Margin::new(1, 1)),
        );
    } else {
        let state = match screen {
            Screen::Archive => &mut app.archive_list,
            Screen::Memo => &mut app.memo_list,
            _ => &mut app.done_list,
        };
        f.render_stateful_widget(widget, area, state);
    }
}

/// 一覧の1行。ビジュアルモードで選んでいれば色を付ける
fn item(spans: Vec<Span<'static>>, in_visual: bool) -> ListItem<'static> {
    let item = ListItem::new(Line::from(spans));
    if in_visual {
        item.style(Style::new().bg(VISUAL).fg(Color::White))
    } else {
        item
    }
}

/// 名前。検索語に合うところを目立たせる (vim の hlsearch)
fn title_spans(title: String, query: Option<&str>) -> Vec<Span<'static>> {
    let ranges = query.map(|q| matches(&title, q)).unwrap_or_default();
    if ranges.is_empty() {
        return vec![Span::raw(title)];
    }
    let hit = Style::new().fg(Color::Black).bg(TODAY);
    let mut spans = Vec::new();
    let mut last = 0;
    for r in ranges {
        if r.start > last {
            spans.push(Span::raw(title[last..r.start].to_string()));
        }
        spans.push(Span::styled(title[r.clone()].to_string(), hit));
        last = r.end;
    }
    if last < title.len() {
        spans.push(Span::raw(title[last..].to_string()));
    }
    spans
}

/// RFC3339 を手元の日付 (YYYY-MM-DD) にする。読めなければ ----------
fn local_date(d: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(d)
        .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|_| "----------".into())
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
    let sub = if w.kind.is_list() {
        format!("{} / {}", w.kind.label(), w.quadrant_label())
    } else {
        format!("{} / {}", w.kind.label(), local_date(&w.created_at))
    };
    let mut lines = vec![
        Line::from(Span::styled(w.title.clone(), Style::new().bold())),
        Line::from(Span::styled(sub, Style::new().fg(ACCENT))),
    ];
    if let Some(d) = w.due() {
        let now = chrono::Local::now().naive_local();
        let style = match due::state(d, now) {
            due::State::Over => Style::new().fg(OVER).bold(),
            due::State::Today => Style::new().fg(TODAY),
            due::State::Later => Style::new(),
        };
        lines.push(Line::from(vec![
            Span::styled("日時 ", Style::new().fg(DIM)),
            Span::styled(due::format(d, now.date()), style),
        ]));
    }
    lines.push(Line::from(""));
    if w.notes.is_empty() {
        lines.push(Line::from("メモなし".fg(DIM)));
    } else {
        lines.extend(w.notes.lines().map(|l| Line::from(l.to_string())));
    }
    lines
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    // `/` `:` を打っている間は vim のコマンドラインと同じく1行まるごと使う
    match &app.mode {
        Mode::Search(input) => return draw_field(f, "/", input, true, area),
        Mode::Command(input) => return draw_field(f, ":", input, true, area),
        _ => {}
    }
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
    if matches!(app.mode, Mode::Visual { .. }) {
        spans.push(Span::styled("-- VISUAL -- ", Style::new().fg(ACCENT).bold()));
    }
    match &app.message {
        Some(m) => spans.push(Span::styled(m.clone(), Style::new().fg(ACCENT))),
        None => spans.push(Span::styled(footer(app), Style::new().fg(DIM))),
    }
    // 打ちかけのカウントと前置キーは右端に出す (vim の showcmd)
    let pending = app.pending.show();
    let [left, right] =
        Layout::horizontal([Constraint::Min(0), Constraint::Length(pending.width() as u16 + 1)])
            .areas(area);
    f.render_widget(Paragraph::new(Line::from(spans)), left);
    f.render_widget(Paragraph::new(pending).alignment(Alignment::Right), right);
}

/// フッターに出すキー。全部は `?` で出す
fn footer(app: &App) -> &'static str {
    let visual = matches!(app.mode, Mode::Visual { .. });
    match (app.screen, visual) {
        (Screen::List, true) => {
            "j/k:範囲 t:やった a:しまう d:削除 J/K:並べ替え H/L:左右へ X:もう一方へ Esc:やめる"
        }
        (Screen::Done, true) => "j/k:範囲 p:リストに戻す d:削除 Esc:やめる",
        (Screen::Archive, true) => "j/k:範囲 p:リストに戻す t:やった d:削除 Esc:やめる",
        (Screen::Memo, true) => "j/k:範囲 d:削除 Esc:やめる",
        (Screen::List, false) if app.kind == Kind::Task => {
            "o:追加 c:編集 s:日時 m:メモ t:やった a:しまう dd:削除 p:貼る u:取消 HJKL:動かす X:Wantへ v:選択 /:検索 ?:ヘルプ"
        }
        (Screen::List, false) => {
            "o:追加 c:編集 m:メモ t:やった a:しまう dd:削除 p:貼る u:取消 HJKL:動かす X:Mustへ v:選択 /:検索 ?:ヘルプ"
        }
        (Screen::Done, false) => "m:メモ p:やったを取り消す dd:削除 u:取消 v:選択 /:検索 ?:ヘルプ",
        (Screen::Archive, false) => "m:メモ p:リストに戻す t:やった dd:削除 u:取消 v:選択 /:検索 ?:ヘルプ",
        (Screen::Memo, false) => "o:追加 Enter:vimで開く c:名前 dd:削除 p:貼る u:取消 /:検索 ?:ヘルプ",
    }
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

fn popup_block(title: &str) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT))
        .title(Span::styled(format!(" {title} "), Style::new().fg(ACCENT).bold()))
}

/// 2択の表示。選ばれているほうを反転する。`focused` ならこの欄にカーソルがある
fn choice(label: &str, a: &str, b: &str, first: bool, focused: bool) -> Line<'static> {
    let on = if focused {
        Style::new().bg(ACCENT).fg(Color::Black).bold()
    } else {
        Style::new().fg(ACCENT)
    };
    let off = Style::new().fg(DIM);
    let head = if focused { Style::new().fg(ACCENT).bold() } else { Style::new().fg(DIM) };
    Line::from(vec![
        Span::styled(format!("{} {label}  ", if focused { "▶" } else { " " }), head),
        Span::styled(format!(" {a} "), if first { on } else { off }),
        Span::raw(" "),
        Span::styled(format!(" {b} "), if first { off } else { on }),
    ])
}

fn draw_popup(f: &mut Frame, app: &App) {
    let area = f.area();
    match &app.mode {
        Mode::Normal | Mode::Visual { .. } | Mode::Search(_) | Mode::Command(_) => {}
        Mode::Add { input, at } => {
            let r = popup_area(area, 60, 5);
            f.render_widget(Clear, r);
            let memo = app.screen == Screen::Memo;
            let kind = if memo { Kind::Memo } else { app.kind };
            let block = popup_block(&format!("{} に追加", kind.label()));
            let inner = block.inner(r);
            f.render_widget(block, r);
            let [line, help] =
                Layout::vertical([Constraint::Length(2), Constraint::Length(1)]).areas(inner);
            draw_field(f, "名前 ", input, true, line);
            let h = if memo {
                "Enter/Esc:作って本文をエディタで開く  Ctrl+C:やめる".to_string()
            } else {
                format!("Enter/Esc:「{}」の {} 番目に追加  Ctrl+C:やめる", app.cur_q().label(app.kind), at + 1)
            };
            f.render_widget(Paragraph::new(h.fg(DIM)), help);
        }
        Mode::Edit(e) => draw_edit(f, e, area),
        Mode::Help(scroll) => draw_help(f, *scroll, area),
    }
}

fn draw_edit(f: &mut Frame, e: &Edit, area: Rect) {
    let memo = e.kind == Kind::Memo;
    let has_due = e.kind.has_due();
    let height = 16 + u16::from(has_due) * 2 + u16::from(e.error.is_some()) - u16::from(memo) * 3;
    let r = popup_area(area, 70, height);
    f.render_widget(Clear, r);
    let block = popup_block(&format!("{} を編集", e.kind.label()));
    let inner = block.inner(r);
    f.render_widget(block, r);

    let mut rows = vec![Constraint::Length(2)];
    if !memo {
        rows.push(Constraint::Length(3));
    }
    if has_due {
        rows.push(Constraint::Length(2));
    }
    if e.error.is_some() {
        rows.push(Constraint::Length(1));
    }
    rows.push(Constraint::Min(1));
    rows.push(Constraint::Length(1));
    let rects = Layout::vertical(rows).split(inner);
    let mut next = rects.iter().copied();
    let mut take = || next.next().unwrap_or(inner);

    // `:` を打っている間はカーソルをそちらに置く
    let at = |field: EditField| e.cmd.is_none() && e.field == field;
    draw_field(f, "名前 ", &e.title, at(EditField::Title), take());

    if !memo {
        // 軸の名前は長さが違うので、ラベルの幅を揃えてから並べる
        let labels = [e.kind.axis(), "clau度"];
        let w = labels.iter().map(|l| l.width()).max().unwrap_or(0);
        f.render_widget(
            Paragraph::new(vec![
                choice(&pad(labels[0], w), "高", "低", e.axis_hi, e.field == EditField::Axis),
                choice(&pad(labels[1], w), "高", "低", e.clau, e.field == EditField::Clau),
            ]),
            take(),
        );
    }

    if has_due {
        let [line, hint] =
            Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).areas(take());
        draw_field(f, "日時 ", &e.due, at(EditField::Due), line);
        f.render_widget(Paragraph::new(format!("     {}", due::HINT).fg(DIM)), hint);
    }
    if let Some(err) = &e.error {
        f.render_widget(Paragraph::new(truncate(err, 66).fg(OVER)), take());
    }

    let label = "メモ ";
    let indent = " ".repeat(label.width());
    let notes: Vec<Line> = if e.notes.is_empty() {
        vec![Line::from(vec![Span::styled(label, Style::new().fg(DIM)), "なし".fg(DIM)])]
    } else {
        e.notes
            .lines()
            .enumerate()
            .map(|(i, l)| {
                let head = if i == 0 { label.to_string() } else { indent.clone() };
                Line::from(vec![Span::styled(head, Style::new().fg(DIM)), Span::raw(l.to_string())])
            })
            .collect()
    };
    f.render_widget(Paragraph::new(notes), take());

    let help = take();
    if let Some(cmd) = &e.cmd {
        return draw_field(f, ":", cmd, true, help);
    }
    let line = if e.inserting() {
        Line::from(vec![
            Span::styled("-- INSERT -- ", Style::new().fg(ACCENT).bold()),
            "Esc/jj:ノーマル  ↑↓:欄  Enter:保存  Tab:メモ".fg(DIM),
        ])
    } else if e.typing() {
        Line::from("i/a:挿入  j/k:欄  Enter/:w:保存  Esc/q:やめる  Tab:メモ".fg(DIM))
    } else {
        Line::from("h/l:高低  j/k:欄  Enter/:w:保存  Esc/q:やめる  Tab:メモ".fg(DIM))
    };
    f.render_widget(Paragraph::new(line), help);
}

fn help_area(area: Rect) -> Rect {
    popup_area(area, 92, help_lines() as u16 + 2)
}

/// `?` のキー一覧
fn draw_help(f: &mut Frame, scroll: u16, area: Rect) {
    let r = help_area(area);
    f.render_widget(Clear, r);
    let key_w = HELP.iter().flat_map(|(_, keys)| keys.iter().map(|(k, _)| k.width())).max().unwrap_or(0);
    let mut lines = Vec::new();
    for (head, keys) in HELP {
        lines.push(Line::from(Span::styled(head.to_string(), Style::new().fg(ACCENT).bold())));
        for (k, desc) in keys.iter() {
            lines.push(Line::from(vec![
                Span::styled(format!("  {}  ", pad(k, key_w)), Style::new().bold()),
                Span::raw(desc.to_string()),
            ]));
        }
        lines.push(Line::from(""));
    }
    let block = popup_block("キー一覧  j/k:スクロール  q:閉じる");
    f.render_widget(Paragraph::new(lines).block(block).scroll((scroll, 0)), r);
}

/// ラベル付きの1行入力欄。カーソルのある欄にだけカーソルを置く
fn draw_field(f: &mut Frame, label: &str, input: &TextInput, focused: bool, area: Rect) {
    let label_w = label.width() as u16;
    let head = if focused { Style::new().fg(ACCENT) } else { Style::new().fg(DIM) };
    let line = Line::from(vec![
        Span::styled(label.to_string(), head),
        Span::raw(input.text.clone()),
    ]);
    f.render_widget(Paragraph::new(line), area);
    if focused {
        // 長い行はスクロールさせず、見える範囲に収める
        let col = input.before_cursor().width() as u16;
        let x = (area.x + label_w + col).min(area.right().saturating_sub(1));
        f.set_cursor_position(Position::new(x, area.y));
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
