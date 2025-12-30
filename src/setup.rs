use crate::database::Database;
use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame, Terminal,
};
use std::io;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq)]
enum SetupStep {
    Welcome,
    Token,
    Channels,
    Complete,
}

pub struct SetupWizard {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    db: Arc<Database>,
    step: SetupStep,
    token_input: String,
    channels_input: String,
    cursor_visible: bool,
    error_message: Option<String>,
}

impl SetupWizard {
    pub fn new(db: Arc<Database>) -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        Ok(Self {
            terminal,
            db,
            step: SetupStep::Welcome,
            token_input: String::new(),
            channels_input: String::new(),
            cursor_visible: true,
            error_message: None,
        })
    }

    pub fn run(&mut self) -> Result<bool> {
        loop {
            self.draw()?;

            if event::poll(Duration::from_millis(500))? {
                if let Event::Key(key) = event::read()? {
                    if key.kind == KeyEventKind::Press {
                        match self.step {
                            SetupStep::Welcome => {
                                match key.code {
                                    KeyCode::Enter => self.step = SetupStep::Token,
                                    KeyCode::Esc => {
                                        self.cleanup()?;
                                        return Ok(false);
                                    }
                                    _ => {}
                                }
                            }
                            SetupStep::Token => {
                                match key.code {
                                    KeyCode::Enter => {
                                        if self.validate_token() {
                                            self.step = SetupStep::Channels;
                                            self.error_message = None;
                                        }
                                    }
                                    KeyCode::Backspace => {
                                        self.token_input.pop();
                                        self.error_message = None;
                                    }
                                    KeyCode::Char(c) => {
                                        self.token_input.push(c);
                                        self.error_message = None;
                                    }
                                    KeyCode::Esc => {
                                        self.cleanup()?;
                                        return Ok(false);
                                    }
                                    _ => {}
                                }
                            }
                            SetupStep::Channels => {
                                match key.code {
                                    KeyCode::Enter => {
                                        if self.validate_and_save_channels() {
                                            self.step = SetupStep::Complete;
                                            self.error_message = None;
                                        }
                                    }
                                    KeyCode::Backspace => {
                                        self.channels_input.pop();
                                        self.error_message = None;
                                    }
                                    KeyCode::Char(c) if c.is_ascii_digit() || c == ',' || c == ' ' => {
                                        self.channels_input.push(c);
                                        self.error_message = None;
                                    }
                                    KeyCode::Esc => {
                                        self.cleanup()?;
                                        return Ok(false);
                                    }
                                    _ => {}
                                }
                            }
                            SetupStep::Complete => {
                                match key.code {
                                    KeyCode::Enter => {
                                        self.cleanup()?;
                                        return Ok(true);
                                    }
                                    KeyCode::Esc => {
                                        self.cleanup()?;
                                        return Ok(true);
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            } else {
                self.cursor_visible = !self.cursor_visible;
            }
        }
    }

    fn validate_token(&mut self) -> bool {
        let token = self.token_input.trim();
        if token.is_empty() {
            self.error_message = Some("Token cannot be empty".to_string());
            return false;
        }
        if token.len() < 50 {
            self.error_message = Some("Token appears too short".to_string());
            return false;
        }
        if let Err(e) = self.db.save_token(token) {
            self.error_message = Some(format!("Failed to save: {}", e));
            return false;
        }
        true
    }

    fn validate_and_save_channels(&mut self) -> bool {
        let input = self.channels_input.trim();
        if input.is_empty() {
            self.error_message = Some("Enter at least one channel ID".to_string());
            return false;
        }

        let channels: Result<Vec<u64>, _> = input
            .split(|c| c == ',' || c == ' ')
            .filter(|s| !s.is_empty())
            .map(|s| s.trim().parse::<u64>())
            .collect();

        match channels {
            Ok(ids) if ids.is_empty() => {
                self.error_message = Some("Enter at least one channel ID".to_string());
                false
            }
            Ok(ids) => {
                if let Err(e) = self.db.save_channels(&ids) {
                    self.error_message = Some(format!("Failed to save: {}", e));
                    return false;
                }
                true
            }
            Err(_) => {
                self.error_message = Some("Invalid channel ID format".to_string());
                false
            }
        }
    }

    fn draw(&mut self) -> Result<()> {
        let step = self.step;
        let token_input = self.token_input.clone();
        let channels_input = self.channels_input.clone();
        let cursor_visible = self.cursor_visible;
        let error_message = self.error_message.clone();

        self.terminal.draw(|frame| {
            let size = frame.size();
            
            let area = centered_rect(60, 50, size);
            frame.render_widget(Clear, area);

            match step {
                SetupStep::Welcome => Self::render_welcome(frame, area),
                SetupStep::Token => Self::render_token_input(frame, area, &token_input, cursor_visible, &error_message),
                SetupStep::Channels => Self::render_channels_input(frame, area, &channels_input, cursor_visible, &error_message),
                SetupStep::Complete => Self::render_complete(frame, area),
            }
        })?;

        Ok(())
    }

    fn render_welcome(frame: &mut Frame, area: Rect) {
        let text = vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("  Welcome to ", Style::default().fg(Color::White)),
                Span::styled("Mudae Selfbot", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(""),
            Line::from(Span::styled("  This wizard will help you set up the bot.", Style::default().fg(Color::Gray))),
            Line::from(""),
            Line::from(Span::styled("  You will need:", Style::default().fg(Color::White))),
            Line::from(Span::styled("    • Your Discord user token", Style::default().fg(Color::Cyan))),
            Line::from(Span::styled("    • Channel IDs to monitor", Style::default().fg(Color::Cyan))),
            Line::from(""),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Press ", Style::default().fg(Color::DarkGray)),
                Span::styled("Enter", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::styled(" to continue or ", Style::default().fg(Color::DarkGray)),
                Span::styled("Esc", Style::default().fg(Color::Red)),
                Span::styled(" to exit", Style::default().fg(Color::DarkGray)),
            ]),
        ];

        let paragraph = Paragraph::new(text).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Magenta))
                .title(" Setup Wizard ")
                .title_style(Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
        );

        frame.render_widget(paragraph, area);
    }

    fn render_token_input(frame: &mut Frame, area: Rect, input: &str, cursor: bool, error: &Option<String>) {
        let cursor_char = if cursor { "▌" } else { " " };
        let display_token = if input.len() > 20 {
            format!("{}...{}", &input[..10], &input[input.len()-10..])
        } else if input.is_empty() {
            String::new()
        } else {
            "*".repeat(input.len().min(30))
        };

        let mut text = vec![
            Line::from(""),
            Line::from(Span::styled("  Step 1: Discord Token", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(Span::styled("  Enter your Discord user token below.", Style::default().fg(Color::Gray))),
            Line::from(Span::styled("  (Token is hidden for security)", Style::default().fg(Color::DarkGray))),
            Line::from(""),
            Line::from(vec![
                Span::styled("  > ", Style::default().fg(Color::Yellow)),
                Span::styled(&display_token, Style::default().fg(Color::White)),
                Span::styled(cursor_char, Style::default().fg(Color::Yellow)),
            ]),
            Line::from(""),
        ];

        if let Some(err) = error {
            text.push(Line::from(Span::styled(format!("  ✗ {}", err), Style::default().fg(Color::Red))));
        }

        text.push(Line::from(""));
        text.push(Line::from(vec![
            Span::styled("  Press ", Style::default().fg(Color::DarkGray)),
            Span::styled("Enter", Style::default().fg(Color::Green)),
            Span::styled(" to continue", Style::default().fg(Color::DarkGray)),
        ]));

        let paragraph = Paragraph::new(text).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Setup - Token ")
                .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        );

        frame.render_widget(paragraph, area);
    }

    fn render_channels_input(frame: &mut Frame, area: Rect, input: &str, cursor: bool, error: &Option<String>) {
        let cursor_char = if cursor { "▌" } else { " " };

        let mut text = vec![
            Line::from(""),
            Line::from(Span::styled("  Step 2: Channel IDs", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(Span::styled("  Enter channel IDs (comma or space separated)", Style::default().fg(Color::Gray))),
            Line::from(Span::styled("  Example: 123456789, 987654321", Style::default().fg(Color::DarkGray))),
            Line::from(""),
            Line::from(vec![
                Span::styled("  > ", Style::default().fg(Color::Yellow)),
                Span::styled(input, Style::default().fg(Color::White)),
                Span::styled(cursor_char, Style::default().fg(Color::Yellow)),
            ]),
            Line::from(""),
        ];

        if let Some(err) = error {
            text.push(Line::from(Span::styled(format!("  ✗ {}", err), Style::default().fg(Color::Red))));
        }

        text.push(Line::from(""));
        text.push(Line::from(vec![
            Span::styled("  Press ", Style::default().fg(Color::DarkGray)),
            Span::styled("Enter", Style::default().fg(Color::Green)),
            Span::styled(" to continue", Style::default().fg(Color::DarkGray)),
        ]));

        let paragraph = Paragraph::new(text).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Setup - Channels ")
                .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        );

        frame.render_widget(paragraph, area);
    }

    fn render_complete(frame: &mut Frame, area: Rect) {
        let text = vec![
            Line::from(""),
            Line::from(Span::styled("  ✓ Setup Complete!", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(Span::styled("  Your configuration has been saved.", Style::default().fg(Color::Gray))),
            Line::from(""),
            Line::from(Span::styled("  You can update these settings anytime by:", Style::default().fg(Color::White))),
            Line::from(Span::styled("    • Pressing 's' in the main dashboard", Style::default().fg(Color::Cyan))),
            Line::from(""),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Press ", Style::default().fg(Color::DarkGray)),
                Span::styled("Enter", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::styled(" to start the bot", Style::default().fg(Color::DarkGray)),
            ]),
        ];

        let paragraph = Paragraph::new(text).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green))
                .title(" Setup Complete ")
                .title_style(Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        );

        frame.render_widget(paragraph, area);
    }

    fn cleanup(&mut self) -> Result<()> {
        disable_raw_mode()?;
        execute!(self.terminal.backend_mut(), LeaveAlternateScreen)?;
        self.terminal.show_cursor()?;
        Ok(())
    }
}

impl Drop for SetupWizard {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
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

pub fn run_setup(db: Arc<Database>) -> Result<bool> {
    let mut wizard = SetupWizard::new(db)?;
    wizard.run()
}
