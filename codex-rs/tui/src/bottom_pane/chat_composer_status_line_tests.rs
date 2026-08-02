use pretty_assertions::assert_eq;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::text::Line;

use super::ChatComposer;
use super::CollaborationModeIndicator;
use crate::app_event::AppEvent;
use crate::app_event_sender::AppEventSender;
use crate::render::renderable::Renderable;

fn test_composer() -> ChatComposer {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
    ChatComposer::new(
        /*has_input_focus*/ true,
        AppEventSender::new(tx),
        /*enhanced_keys_supported*/ false,
        "Ask Codex to do anything".to_string(),
        /*disable_paste_burst*/ false,
    )
}

fn status_text(lines: &[Line<'static>]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect()
        })
        .collect()
}

fn snapshot_composer(name: &str, width: u16, composer: &ChatComposer) {
    let height = composer.desired_height(width);
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("terminal");
    terminal
        .draw(|frame| composer.render(frame.area(), frame.buffer_mut()))
        .expect("draw composer");
    insta::assert_snapshot!(name, terminal.backend());
}

#[test]
fn status_line_rows_are_bounded_at_the_presentation_boundary() {
    let mut composer = test_composer();

    composer.set_status_lines(vec![
        "one".into(),
        "two".into(),
        "three".into(),
        "four".into(),
    ]);

    assert_eq!(
        status_text(&composer.footer.status_line_lines),
        vec!["one", "two", "three"]
    );
}

#[test]
fn multiline_status_line_reserves_rows_and_keeps_right_badges() {
    let mut composer = test_composer();
    composer.set_status_line_enabled(/*enabled*/ true);
    composer.set_collaboration_mode_indicator(Some(CollaborationModeIndicator::Plan));
    composer.set_ide_context_active(/*active*/ true);
    composer.set_status_lines(vec![
        "first row is independently truncated at the terminal edge".into(),
        "second row".into(),
        "third row is truncated before the badges instead of hiding them".into(),
    ]);

    snapshot_composer(
        "multiline_status_line_reserves_rows_and_keeps_right_badges",
        /*width*/ 52,
        &composer,
    );
}

#[test]
fn instructional_footer_suppresses_all_status_rows() {
    let mut composer = test_composer();
    composer.set_status_line_enabled(/*enabled*/ true);
    composer.set_status_lines(vec!["one".into(), "two".into(), "three".into()]);
    composer.set_task_running(/*running*/ true);
    composer.set_text_content("queued draft".to_string(), Vec::new(), Vec::new());

    snapshot_composer(
        "instructional_footer_suppresses_all_status_rows",
        /*width*/ 72,
        &composer,
    );
}
