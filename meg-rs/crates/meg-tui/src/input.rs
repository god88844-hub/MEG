//! Windows console records do not preserve bracketed-paste markers. Coalesce
//! rapid text records before dispatch so embedded Enter keys cannot submit.
//! Human key events and explicit crossterm paste events keep their normal path.
use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};
use std::{io, time::Duration};

pub fn read_terminal_batch(navigation_fields: bool) -> io::Result<Vec<Event>> {
    if cfg!(windows) {
        read_batch(
            crossterm::event::poll,
            crossterm::event::read,
            navigation_fields,
        )
    } else {
        Ok(vec![crossterm::event::read()?])
    }
}

fn plain_text(event: &Event) -> Option<char> {
    let Event::Key(key) = event else {
        return None;
    };
    if key.kind == KeyEventKind::Release {
        return None;
    }
    // ConPTY represents a pasted LF as Ctrl+Enter, whereas CR uses Enter.
    if key.code == KeyCode::Enter
        && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::CONTROL)
    {
        return Some('\n');
    }
    let altgr = key
        .modifiers
        .contains(KeyModifiers::CONTROL | KeyModifiers::ALT);
    if key
        .modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        && !altgr
    {
        return None;
    }
    match key.code {
        KeyCode::Char(ch) => Some(ch),
        KeyCode::Tab if key.modifiers.is_empty() => Some('\t'),
        _ => None,
    }
}

fn read_batch(
    mut poll: impl FnMut(Duration) -> io::Result<bool>,
    mut read: impl FnMut() -> io::Result<Event>,
    navigation_fields: bool,
) -> io::Result<Vec<Event>> {
    let first = read()?;
    // Enter alone always remains a deliberate submit, never a paste starter.
    if !matches!(&first,Event::Key(key) if matches!(key.code,KeyCode::Char(_))&&plain_text(&first).is_some())
    {
        return Ok(vec![first]);
    }
    let mut text = String::from(plain_text(&first).unwrap());
    let mut count = 1usize;
    let mut next = None;
    loop {
        let interval = Duration::from_millis(if count == 1 { 8 } else { 60 });
        if !poll(interval)? {
            break;
        }
        let event = read()?;
        if navigation_fields && matches!(&event, Event::Key(key) if key.code == KeyCode::Tab) {
            next = Some(event);
            break;
        }
        if matches!(&event,Event::Key(key)if key.kind==KeyEventKind::Release) {
            continue;
        }
        if let Some(ch) = plain_text(&event) {
            count += 1;
            if text.len() <= 1024 * 1024 {
                text.push(ch);
            }
        } else {
            next = Some(event);
            break;
        }
    }
    let mut events = vec![if count > 1 { Event::Paste(text) } else { first }];
    if let Some(next) = next {
        events.push(next);
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;
    use std::{collections::VecDeque, sync::Mutex};
    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }
    fn batch(events: Vec<Event>) -> Vec<Event> {
        let input = Mutex::new(VecDeque::from(events));
        read_batch(
            |_| Ok(!input.lock().unwrap().is_empty()),
            || Ok(input.lock().unwrap().pop_front().unwrap()),
            false,
        )
        .unwrap()
    }
    #[test]
    fn multiline_console_records_become_one_editable_paste() {
        assert_eq!(
            batch(vec![
                key(KeyCode::Char('a')),
                key(KeyCode::Enter),
                key(KeyCode::Char('世'))
            ]),
            vec![Event::Paste("a\n世".into())]
        );
    }
    #[test]
    fn conpty_ctrl_enter_inside_text_is_a_pasted_line_feed() {
        assert_eq!(
            batch(vec![
                key(KeyCode::Char('a')),
                Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL)),
                key(KeyCode::Char('b'))
            ]),
            vec![Event::Paste("a\nb".into())]
        );
    }
    #[test]
    fn deliberate_enter_and_escape_keep_their_actions() {
        assert_eq!(batch(vec![key(KeyCode::Enter)]), vec![key(KeyCode::Enter)]);
        assert_eq!(
            batch(vec![key(KeyCode::Char('a')), key(KeyCode::Esc)]),
            vec![key(KeyCode::Char('a')), key(KeyCode::Esc)]
        );
    }
    #[test]
    fn modified_shortcuts_are_never_swallowed_by_a_paste() {
        let stop = Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert_eq!(
            batch(vec![
                key(KeyCode::Char('a')),
                key(KeyCode::Char('b')),
                stop.clone()
            ]),
            vec![Event::Paste("ab".into()), stop]
        );
    }

    #[test]
    fn form_tab_remains_navigation_after_fast_protocol_space() {
        let mut input = VecDeque::from([key(KeyCode::Char(' ')), key(KeyCode::Tab)]);
        let result = read_batch(|_| Ok(true), || Ok(input.pop_front().unwrap()), true).unwrap();
        assert_eq!(result, vec![key(KeyCode::Char(' ')), key(KeyCode::Tab)]);
    }
}
