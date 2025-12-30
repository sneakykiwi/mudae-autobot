use crate::config::Config;
use crate::database::{ChannelInfo, Database};
use crate::search::{SearchRequest, SearchRequestSender, SearchResult};
use crate::stats::{ChannelActivity, ConnectionStatus, EventType, Stats};
use crate::wishlist::{WishedCharacter, WishlistManager};
use chrono::Utc;
use tokio::sync::oneshot;
use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame, Terminal,
};
use std::io;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;

#[derive(Clone, PartialEq)]
enum View {
    Dashboard,
    Settings,
    EditToken,
    EditChannels,
    EditRollCommands,
    EditCooldown,
    Wishlist,
    SearchCharacter,
    ConfirmCharacter(SearchResult),
}

#[derive(Clone, Copy, PartialEq)]
enum SettingsItem {
    Token,
    Channels,
    RollCommands,
    Cooldown,
    AutoRoll,
    AutoKakera,
    AutoDaily,
    Wishlist,
    FuzzyMatch,
}

impl SettingsItem {
    fn all() -> &'static [SettingsItem] {
        &[
            SettingsItem::Token,
            SettingsItem::Channels,
            SettingsItem::RollCommands,
            SettingsItem::Cooldown,
            SettingsItem::AutoRoll,
            SettingsItem::AutoKakera,
            SettingsItem::AutoDaily,
            SettingsItem::Wishlist,
            SettingsItem::FuzzyMatch,
        ]
    }

    fn label(&self) -> &'static str {
        match self {
            SettingsItem::Token => "Discord Token",
            SettingsItem::Channels => "Channel IDs",
            SettingsItem::RollCommands => "Roll Commands",
            SettingsItem::Cooldown => "Roll Cooldown (seconds)",
            SettingsItem::AutoRoll => "Auto Roll",
            SettingsItem::AutoKakera => "Auto Kakera React",
            SettingsItem::AutoDaily => "Auto Daily",
            SettingsItem::Wishlist => "Wishlist Enabled",
            SettingsItem::FuzzyMatch => "Fuzzy Match",
        }
    }

    fn is_toggle(&self) -> bool {
        matches!(
            self,
            SettingsItem::AutoRoll
                | SettingsItem::AutoKakera
                | SettingsItem::AutoDaily
                | SettingsItem::Wishlist
                | SettingsItem::FuzzyMatch
        )
    }
}

pub struct Tui {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    stats: Arc<Stats>,
    config: Config,
    db: Arc<Database>,
    wishlist: Arc<WishlistManager>,
    search_tx: SearchRequestSender,
    shutdown_rx: watch::Receiver<bool>,
    channel_infos: Vec<ChannelInfo>,
    client: Option<Arc<crate::client::DiscordClient>>,
    scroll_offset: u16,
    view: View,
    input_buffer: String,
    settings_cursor: usize,
    wishlist_cursor: usize,
    cursor_visible: bool,
    message: Option<(String, bool)>,
    searching: bool,
    pending_search: Option<(String, oneshot::Receiver<Option<SearchResult>>)>,
    pending_channel_refresh: Option<oneshot::Receiver<()>>,
}

impl Tui {
    pub fn new(
        stats: Arc<Stats>,
        config: Config,
        db: Arc<Database>,
        wishlist: Arc<WishlistManager>,
        search_tx: SearchRequestSender,
        shutdown_rx: watch::Receiver<bool>,
        channel_infos: Vec<ChannelInfo>,
        client: Option<Arc<crate::client::DiscordClient>>,
    ) -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        Ok(Self {
            terminal,
            stats,
            config,
            db,
            wishlist,
            search_tx,
            shutdown_rx,
            channel_infos,
            client,
            scroll_offset: 0,
            view: View::Dashboard,
            input_buffer: String::new(),
            settings_cursor: 0,
            wishlist_cursor: 0,
            cursor_visible: true,
            message: None,
            searching: false,
            pending_search: None,
            pending_channel_refresh: None,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            if *self.shutdown_rx.borrow() {
                break;
            }

            self.check_pending_search().await;
            self.check_pending_channel_refresh().await;

            tokio::select! {
                _ = tick.tick() => {
                    self.draw().await?;
                    self.cursor_visible = !self.cursor_visible;
                }
                result = tokio::task::spawn_blocking(|| {
                    if event::poll(Duration::from_millis(0)).unwrap_or(false) {
                        event::read().ok()
                    } else {
                        None
                    }
                }) => {
                    if let Ok(Some(Event::Key(key))) = result {
                        if key.kind == KeyEventKind::Press {
                            let should_quit = match &self.view {
                                View::Dashboard => self.handle_dashboard_input(key.code),
                                View::Settings => { self.handle_settings_input(key.code); false }
                                View::EditToken => { self.handle_edit_token_input(key.code); false }
                                View::EditChannels => { self.handle_edit_channels_input(key.code); false }
                                View::EditRollCommands => { self.handle_edit_roll_commands_input(key.code); false }
                                View::EditCooldown => { self.handle_edit_cooldown_input(key.code); false }
                                View::Wishlist => { self.handle_wishlist_input(key.code).await; false }
                                View::SearchCharacter => { self.handle_search_input(key.code).await; false }
                                View::ConfirmCharacter(_) => { self.handle_confirm_input(key.code).await; false }
                            };
                            if should_quit {
                                break;
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    async fn check_pending_search(&mut self) {
        if let Some((query, mut rx)) = self.pending_search.take() {
            match rx.try_recv() {
                Ok(Some(result)) => {
                    self.searching = false;
                    self.view = View::ConfirmCharacter(result);
                    self.message = None;
                }
                Ok(None) => {
                    self.searching = false;
                    self.message = Some((format!("No character found for '{}'", query), false));
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    self.pending_search = Some((query, rx));
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    self.searching = false;
                    self.message = Some(("Search failed - channel closed".to_string(), false));
                }
            }
        }
    }

    async fn check_pending_channel_refresh(&mut self) {
        if let Some(mut rx) = self.pending_channel_refresh.take() {
            match rx.try_recv() {
                Ok(()) => {
                    if let Ok(updated_infos) = self.db.get_channels_with_names() {
                        self.channel_infos = updated_infos;
                    }
                }
                Err(oneshot::error::TryRecvError::Empty) => {
                    self.pending_channel_refresh = Some(rx);
                }
                Err(oneshot::error::TryRecvError::Closed) => {}
            }
        }
    }

    fn handle_dashboard_input(&mut self, key: KeyCode) -> bool {
        match key {
            KeyCode::Char('q') | KeyCode::Esc => return true,
            KeyCode::Char('s') => {
                self.view = View::Settings;
                self.settings_cursor = 0;
                self.message = None;
            }
            KeyCode::Char('w') => {
                self.view = View::Wishlist;
                self.wishlist_cursor = 0;
                self.message = None;
            }
            KeyCode::Char('p') | KeyCode::Char(' ') => {
                self.stats.toggle_paused();
            }
            KeyCode::Up => {
                self.scroll_offset = self.scroll_offset.saturating_add(1);
            }
            KeyCode::Down => {
                self.scroll_offset = self.scroll_offset.saturating_sub(1);
            }
            _ => {}
        }
        false
    }

    fn handle_settings_input(&mut self, key: KeyCode) {
        let items = SettingsItem::all();
        match key {
            KeyCode::Esc => {
                self.view = View::Dashboard;
                self.message = None;
            }
            KeyCode::Up => {
                if self.settings_cursor > 0 {
                    self.settings_cursor -= 1;
                }
            }
            KeyCode::Down => {
                if self.settings_cursor < items.len() - 1 {
                    self.settings_cursor += 1;
                }
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                let item = items[self.settings_cursor];
                if item.is_toggle() {
                    self.toggle_setting(item);
                } else {
                    match item {
                        SettingsItem::Token => {
                            self.view = View::EditToken;
                            self.input_buffer.clear();
                            self.message = None;
                        }
                        SettingsItem::Channels => {
                            self.view = View::EditChannels;
                            self.input_buffer = self.channel_infos
                                .iter()
                                .map(|c| c.id.to_string())
                                .collect::<Vec<_>>()
                                .join(", ");
                            self.message = None;
                        }
                        SettingsItem::RollCommands => {
                            self.view = View::EditRollCommands;
                            self.input_buffer = self.config.roll_commands.join(", ");
                            self.message = None;
                        }
                        SettingsItem::Cooldown => {
                            self.view = View::EditCooldown;
                            self.input_buffer = self.config.roll_cooldown_seconds.to_string();
                            self.message = None;
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn toggle_setting(&mut self, item: SettingsItem) {
        match item {
            SettingsItem::AutoRoll => self.config.auto_roll = !self.config.auto_roll,
            SettingsItem::AutoKakera => self.config.auto_react_kakera = !self.config.auto_react_kakera,
            SettingsItem::AutoDaily => self.config.auto_daily = !self.config.auto_daily,
            SettingsItem::Wishlist => self.config.wishlist_enabled = !self.config.wishlist_enabled,
            SettingsItem::FuzzyMatch => self.config.fuzzy_match = !self.config.fuzzy_match,
            _ => return,
        }
        if let Err(e) = self.config.save_to_db(self.db.as_ref()) {
            self.message = Some((format!("Error: {}", e), false));
        } else {
            self.message = Some(("Setting saved!".to_string(), true));
        }
    }

    fn handle_edit_token_input(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc => {
                self.view = View::Settings;
                self.input_buffer.clear();
            }
            KeyCode::Enter => {
                if self.input_buffer.is_empty() {
                    self.message = Some(("Token cannot be empty".to_string(), false));
                } else if let Err(e) = self.db.save_token(&self.input_buffer) {
                    self.message = Some((format!("Error: {}", e), false));
                } else {
                    self.message = Some(("Token saved! Restart to apply.".to_string(), true));
                    self.view = View::Settings;
                    self.input_buffer.clear();
                }
            }
            KeyCode::Backspace => {
                self.input_buffer.pop();
            }
            KeyCode::Char(c) => {
                self.input_buffer.push(c);
            }
            _ => {}
        }
    }

    fn handle_edit_channels_input(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc => {
                self.view = View::Settings;
                self.input_buffer.clear();
            }
            KeyCode::Enter => {
                let channels: Result<Vec<u64>, _> = self.input_buffer
                    .split(|c| c == ',' || c == ' ')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.trim().parse::<u64>())
                    .collect();

                match channels {
                    Ok(ids) if !ids.is_empty() => {
                        if let Err(e) = self.db.save_channels(&ids) {
                            self.message = Some((format!("Error: {}", e), false));
                        } else {
                            self.channel_infos = ids.iter().map(|&id| ChannelInfo {
                                id,
                                name: None,
                                guild: None,
                            }).collect();
                            
                            if let Some(ref client) = self.client {
                                let client_clone = client.clone();
                                let db_clone = self.db.clone();
                                let ids_clone = ids.clone();
                                let (tx, rx) = oneshot::channel();
                                self.pending_channel_refresh = Some(rx);
                                tokio::spawn(async move {
                                    Self::fetch_channel_names(client_clone, db_clone, ids_clone).await;
                                    let _ = tx.send(());
                                });
                            }
                            
                            self.message = Some(("Channels saved! Fetching names...".to_string(), true));
                            self.view = View::Settings;
                            self.input_buffer.clear();
                        }
                    }
                    Ok(_) => {
                        self.message = Some(("Enter at least one channel".to_string(), false));
                    }
                    Err(_) => {
                        self.message = Some(("Invalid channel ID format".to_string(), false));
                    }
                }
            }
            KeyCode::Backspace => {
                self.input_buffer.pop();
            }
            KeyCode::Char(c) if c.is_ascii_digit() || c == ',' || c == ' ' => {
                self.input_buffer.push(c);
            }
            _ => {}
        }
    }

    fn handle_edit_roll_commands_input(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc => {
                self.view = View::Settings;
                self.input_buffer.clear();
            }
            KeyCode::Enter => {
                let commands: Vec<String> = self.input_buffer
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();

                if commands.is_empty() {
                    self.message = Some(("Enter at least one command".to_string(), false));
                } else {
                    self.config.roll_commands = commands;
                    if let Err(e) = self.config.save_to_db(self.db.as_ref()) {
                        self.message = Some((format!("Error: {}", e), false));
                    } else {
                        self.message = Some(("Commands saved!".to_string(), true));
                        self.view = View::Settings;
                        self.input_buffer.clear();
                    }
                }
            }
            KeyCode::Backspace => {
                self.input_buffer.pop();
            }
            KeyCode::Char(c) => {
                self.input_buffer.push(c);
            }
            _ => {}
        }
    }

    fn handle_edit_cooldown_input(&mut self, key: KeyCode) {
        match key {
            KeyCode::Esc => {
                self.view = View::Settings;
                self.input_buffer.clear();
            }
            KeyCode::Enter => {
                match self.input_buffer.parse::<u64>() {
                    Ok(secs) if secs > 0 => {
                        self.config.roll_cooldown_seconds = secs;
                        if let Err(e) = self.config.save_to_db(self.db.as_ref()) {
                            self.message = Some((format!("Error: {}", e), false));
                        } else {
                            self.message = Some(("Cooldown saved!".to_string(), true));
                            self.view = View::Settings;
                            self.input_buffer.clear();
                        }
                    }
                    _ => {
                        self.message = Some(("Enter a valid number".to_string(), false));
                    }
                }
            }
            KeyCode::Backspace => {
                self.input_buffer.pop();
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                self.input_buffer.push(c);
            }
            _ => {}
        }
    }

    async fn handle_wishlist_input(&mut self, key: KeyCode) {
        let chars = tokio::task::block_in_place(|| {
            let rt = tokio::runtime::Handle::current();
            rt.block_on(self.wishlist.get_characters())
        });
        let char_count = chars.len();

        match key {
            KeyCode::Esc => {
                self.view = View::Dashboard;
                self.message = None;
            }
            KeyCode::Char('a') | KeyCode::Char('s') => {
                self.view = View::SearchCharacter;
                self.input_buffer.clear();
                self.searching = false;
                self.message = None;
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if char_count > 0 && self.wishlist_cursor < char_count {
                    let char_name = chars[self.wishlist_cursor].name.clone();
                    let wishlist = self.wishlist.clone();
                    let result = tokio::task::block_in_place(|| {
                        let rt = tokio::runtime::Handle::current();
                        rt.block_on(wishlist.remove_character(&char_name))
                    });
                    match result {
                        Ok(true) => {
                            self.message = Some((format!("Removed '{}'", char_name), true));
                            if self.wishlist_cursor > 0 {
                                self.wishlist_cursor -= 1;
                            }
                        }
                        Ok(false) => {
                            self.message = Some(("Character not found".to_string(), false));
                        }
                        Err(e) => {
                            self.message = Some((format!("Error: {}", e), false));
                        }
                    }
                }
            }
            KeyCode::Up => {
                if self.wishlist_cursor > 0 {
                    self.wishlist_cursor -= 1;
                }
            }
            KeyCode::Down => {
                if self.wishlist_cursor + 1 < char_count {
                    self.wishlist_cursor += 1;
                }
            }
            _ => {}
        }
    }

    async fn handle_search_input(&mut self, key: KeyCode) {
        if self.searching {
            if key == KeyCode::Esc {
                self.searching = false;
                self.pending_search = None;
                self.message = Some(("Search cancelled".to_string(), false));
            }
            return;
        }

        match key {
            KeyCode::Esc => {
                self.view = View::Wishlist;
                self.input_buffer.clear();
                self.message = None;
            }
            KeyCode::Enter => {
                if !self.input_buffer.is_empty() {
                    let query = self.input_buffer.trim().to_string();
                    
                    if let Some(channel_id) = self.channel_infos.first().map(|c| c.id) {
                        self.searching = true;
                        self.message = Some(("Searching...".to_string(), true));
                        
                        let (tx, rx) = oneshot::channel();
                        let request = SearchRequest {
                            query: query.clone(),
                            channel_id,
                            response_tx: tx,
                        };
                        
                        if self.search_tx.send(request).await.is_ok() {
                            self.pending_search = Some((query, rx));
                        } else {
                            self.searching = false;
                            self.message = Some(("Failed to send search request".to_string(), false));
                        }
                    } else {
                        self.message = Some(("No channel configured".to_string(), false));
                    }
                }
            }
            KeyCode::Backspace => {
                self.input_buffer.pop();
            }
            KeyCode::Char(c) => {
                self.input_buffer.push(c);
            }
            _ => {}
        }
    }

    async fn handle_confirm_input(&mut self, key: KeyCode) {
        let result = match &self.view {
            View::ConfirmCharacter(r) => r.clone(),
            _ => return,
        };

        match key {
            KeyCode::Esc | KeyCode::Char('n') => {
                self.view = View::Wishlist;
                self.input_buffer.clear();
                self.message = None;
            }
            KeyCode::Enter | KeyCode::Char('y') => {
                let character = WishedCharacter {
                    name: result.name.clone(),
                    series: Some(result.series.clone()),
                    character_id: None,
                    verified: true,
                    added_date: Utc::now(),
                    notes: None,
                    priority: 0,
                };

                let wishlist = self.wishlist.clone();
                let add_result = tokio::task::block_in_place(|| {
                    let rt = tokio::runtime::Handle::current();
                    rt.block_on(wishlist.add_character(character))
                });

                match add_result {
                    Ok(true) => {
                        self.message = Some((format!("Added '{}' (verified)", result.name), true));
                        self.view = View::Wishlist;
                        self.input_buffer.clear();
                    }
                    Ok(false) => {
                        self.message = Some(("Character already in wishlist".to_string(), false));
                        self.view = View::Wishlist;
                    }
                    Err(e) => {
                        self.message = Some((format!("Error: {}", e), false));
                    }
                }
            }
            _ => {}
        }
    }

    async fn draw(&mut self) -> Result<()> {
        let stats = self.stats.clone();
        let config = self.config.clone();
        let channel_infos = self.db.get_channels_with_names().unwrap_or_else(|_| self.channel_infos.clone());
        let scroll_offset = self.scroll_offset;
        let view = self.view.clone();
        let input_buffer = self.input_buffer.clone();
        let settings_cursor = self.settings_cursor;
        let wishlist_cursor = self.wishlist_cursor;
        let cursor_visible = self.cursor_visible;
        let message = self.message.clone();
        let searching = self.searching;
        
        let connection_status = stats.get_connection_status().await;
        let activity_log = stats.get_activity_log().await;
        let channel_activity = stats.get_channel_activity().await;
        let username = stats.get_username().await;
        let is_paused = stats.is_paused();
        let reset_timer = stats.format_time_until_roll_reset().await;
        let wishlist_chars = self.wishlist.get_characters().await;

        self.terminal.draw(|frame| {
            let size = frame.size();
            
            let main_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3),
                    Constraint::Length(12),
                    Constraint::Min(8),
                    Constraint::Length(1),
                ])
                .split(size);

            Self::render_header(frame, main_chunks[0], &stats, connection_status, username.as_deref(), is_paused);

            let middle_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(main_chunks[1]);

            Self::render_stats_panel(frame, middle_chunks[0], &stats, &reset_timer);
            Self::render_config_panel(frame, middle_chunks[1], &config, &channel_infos);

            let bottom_chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                .split(main_chunks[2]);

            Self::render_activity_log(frame, bottom_chunks[0], &activity_log, scroll_offset);
            Self::render_channel_feed(frame, bottom_chunks[1], &channel_activity);

            Self::render_help_bar(frame, main_chunks[3], is_paused);

            match view {
                View::Settings => {
                    let popup_area = centered_rect(55, 60, size);
                    frame.render_widget(Clear, popup_area);
                    Self::render_settings(frame, popup_area, settings_cursor, &config, &message);
                }
                View::EditToken => {
                    let popup_area = centered_rect(60, 30, size);
                    frame.render_widget(Clear, popup_area);
                    Self::render_text_input(frame, popup_area, "Edit Token", "Enter new Discord token:", &input_buffer, true, cursor_visible, &message);
                }
                View::EditChannels => {
                    let popup_area = centered_rect(60, 30, size);
                    frame.render_widget(Clear, popup_area);
                    Self::render_text_input(frame, popup_area, "Edit Channels", "Enter channel IDs (comma separated):", &input_buffer, false, cursor_visible, &message);
                }
                View::EditRollCommands => {
                    let popup_area = centered_rect(60, 30, size);
                    frame.render_widget(Clear, popup_area);
                    Self::render_text_input(frame, popup_area, "Edit Roll Commands", "Enter commands (comma separated, e.g. $wa, $ha):", &input_buffer, false, cursor_visible, &message);
                }
                View::EditCooldown => {
                    let popup_area = centered_rect(60, 30, size);
                    frame.render_widget(Clear, popup_area);
                    Self::render_text_input(frame, popup_area, "Edit Cooldown", "Enter cooldown in seconds:", &input_buffer, false, cursor_visible, &message);
                }
                View::Wishlist => {
                    let popup_area = centered_rect(70, 80, size);
                    frame.render_widget(Clear, popup_area);
                    Self::render_wishlist(frame, popup_area, &wishlist_chars, wishlist_cursor, &message);
                }
                View::SearchCharacter => {
                    let popup_area = centered_rect(60, 35, size);
                    frame.render_widget(Clear, popup_area);
                    Self::render_search_character(frame, popup_area, &input_buffer, searching, cursor_visible, &message);
                }
                View::ConfirmCharacter(ref result) => {
                    let popup_area = centered_rect(65, 50, size);
                    frame.render_widget(Clear, popup_area);
                    Self::render_confirm_character(frame, popup_area, result, &message);
                }
                View::Dashboard => {}
            }
        })?;

        Ok(())
    }

    fn render_header(frame: &mut Frame, area: Rect, stats: &Stats, status: ConnectionStatus, username: Option<&str>, is_paused: bool) {
        let status_text = match status {
            ConnectionStatus::Connected => ("● CONNECTED", Color::Green),
            ConnectionStatus::Connecting => ("◐ CONNECTING", Color::Yellow),
            ConnectionStatus::Reconnecting => ("◐ RECONNECTING", Color::Yellow),
            ConnectionStatus::Disconnected => ("○ DISCONNECTED", Color::Red),
        };

        let user_display = username.unwrap_or("Not logged in");
        let uptime = stats.format_uptime();

        let mut spans = vec![
            Span::styled(" MUDAE ", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
            Span::styled("│ ", Style::default().fg(Color::DarkGray)),
            Span::styled(user_display, Style::default().fg(Color::Cyan)),
            Span::styled(" │ ", Style::default().fg(Color::DarkGray)),
            Span::styled(status_text.0, Style::default().fg(status_text.1)),
        ];

        if is_paused {
            spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
            spans.push(Span::styled("⏸  PAUSED", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD | Modifier::SLOW_BLINK)));
        }

        spans.push(Span::styled(" │ ", Style::default().fg(Color::DarkGray)));
        spans.push(Span::styled(format!("⏱  {}", uptime), Style::default().fg(Color::White)));

        let header = Paragraph::new(Line::from(spans))
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(if is_paused { Color::Yellow } else { Color::Magenta })),
        );

        frame.render_widget(header, area);
    }

    fn render_help_bar(frame: &mut Frame, area: Rect, is_paused: bool) {
        let help = Paragraph::new(Line::from(vec![
            Span::styled(" ", Style::default()),
            Span::styled("[S]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::styled(" Settings  ", Style::default().fg(Color::DarkGray)),
            Span::styled("[W]", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
            Span::styled(" Wishlist  ", Style::default().fg(Color::DarkGray)),
            Span::styled("[P]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(if is_paused { " Resume  " } else { " Pause  " }, Style::default().fg(Color::DarkGray)),
            Span::styled("[↑↓]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled(" Scroll  ", Style::default().fg(Color::DarkGray)),
            Span::styled("[Q]", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::styled(" Quit", Style::default().fg(Color::DarkGray)),
        ]))
        .style(Style::default().bg(Color::Black));

        frame.render_widget(help, area);
    }

    fn render_stats_panel(frame: &mut Frame, area: Rect, stats: &Stats, reset_timer: &str) {
        let claim_status = if stats.is_claim_available() {
            Span::styled("✓  Available", Style::default().fg(Color::Green))
        } else {
            Span::styled("✗  On Cooldown", Style::default().fg(Color::Red))
        };

        let rolls_remaining = stats.get_rolls_remaining();
        let rolls_status = if rolls_remaining > 0 {
            Span::styled(rolls_remaining.to_string(), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))
        } else {
            Span::styled("0", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD))
        };

        let reset_timer_span = if reset_timer == "Available" || reset_timer == "Unknown" {
            Span::styled(reset_timer.to_string(), Style::default().fg(Color::Yellow))
        } else {
            Span::styled(reset_timer.to_string(), Style::default().fg(Color::Cyan))
        };

        let stats_items = vec![
            ListItem::new(Line::from(vec![
                Span::styled("  Characters Rolled  ", Style::default().fg(Color::White)),
                Span::styled(stats.get_rolled().to_string(), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            ])),
            ListItem::new(Line::from(vec![
                Span::styled("  Characters Claimed ", Style::default().fg(Color::White)),
                Span::styled(stats.get_claimed().to_string(), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            ])),
            ListItem::new(Line::from(vec![
                Span::styled("  Wishlist Matches   ", Style::default().fg(Color::White)),
                Span::styled(stats.get_wishlist_matches().to_string(), Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
            ])),
            ListItem::new(Line::from(vec![
                Span::styled("  Kakera Collected   ", Style::default().fg(Color::White)),
                Span::styled(stats.get_kakera().to_string(), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            ])),
            ListItem::new(Line::from(vec![
                Span::styled("  Rolls Executed     ", Style::default().fg(Color::White)),
                Span::styled(stats.get_rolls_executed().to_string(), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            ])),
            ListItem::new(Line::from("")),
            ListItem::new(Line::from(vec![
                Span::styled("  Rolls Remaining    ", Style::default().fg(Color::White)),
                rolls_status,
            ])),
            ListItem::new(Line::from(vec![
                Span::styled("  Next Roll Reset    ", Style::default().fg(Color::White)),
                reset_timer_span,
            ])),
            ListItem::new(Line::from("")),
            ListItem::new(Line::from(vec![
                Span::styled("  Claim Status       ", Style::default().fg(Color::White)),
                claim_status,
            ])),
            ListItem::new(Line::from(vec![
                Span::styled("  Total Uptime       ", Style::default().fg(Color::White)),
                Span::styled(stats.format_total_uptime(), Style::default().fg(Color::Cyan)),
            ])),
        ];

        let stats_list = List::new(stats_items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(" Statistics ")
                .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        );

        frame.render_widget(stats_list, area);
    }

    fn render_config_panel(frame: &mut Frame, area: Rect, config: &Config, channel_infos: &[ChannelInfo]) {
        let auto_roll_status = Self::status_indicator(config.auto_roll);
        let auto_kakera_status = Self::status_indicator(config.auto_react_kakera);
        let auto_daily_status = Self::status_indicator(config.auto_daily);
        let wishlist_status = Self::status_indicator(config.wishlist_enabled);
        let fuzzy_status = Self::status_indicator(config.fuzzy_match);

        let channels_str = if channel_infos.is_empty() {
            "None".to_string()
        } else if channel_infos.len() <= 2 {
            channel_infos.iter().map(|c| c.display_name()).collect::<Vec<_>>().join(", ")
        } else {
            format!("{} channels", channel_infos.len())
        };

        let config_items = vec![
            ListItem::new(Line::from(vec![
                Span::styled("  Auto Roll          ", Style::default().fg(Color::White)),
                auto_roll_status,
            ])),
            ListItem::new(Line::from(vec![
                Span::styled("  Auto Kakera        ", Style::default().fg(Color::White)),
                auto_kakera_status,
            ])),
            ListItem::new(Line::from(vec![
                Span::styled("  Auto Daily         ", Style::default().fg(Color::White)),
                auto_daily_status,
            ])),
            ListItem::new(Line::from(vec![
                Span::styled("  Wishlist           ", Style::default().fg(Color::White)),
                wishlist_status,
            ])),
            ListItem::new(Line::from(vec![
                Span::styled("  Fuzzy Match        ", Style::default().fg(Color::White)),
                fuzzy_status,
            ])),
            ListItem::new(Line::from("")),
            ListItem::new(Line::from(vec![
                Span::styled("  Roll Commands      ", Style::default().fg(Color::White)),
                Span::styled(config.roll_commands.join(", "), Style::default().fg(Color::Cyan)),
            ])),
            ListItem::new(Line::from(vec![
                Span::styled("  Channels           ", Style::default().fg(Color::White)),
                Span::styled(channels_str, Style::default().fg(Color::Cyan)),
            ])),
        ];

        let config_list = List::new(config_items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
                .title(" Configuration ")
                .title_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        );

        frame.render_widget(config_list, area);
    }

    fn render_activity_log(
        frame: &mut Frame,
        area: Rect,
        events: &[crate::stats::ActivityEvent],
        scroll_offset: u16,
    ) {
        let max_visible = (area.height.saturating_sub(2)) as usize;
        let total_events = events.len();
        
        let start_idx = if total_events > max_visible {
            let max_scroll = total_events.saturating_sub(max_visible);
            let effective_scroll = (scroll_offset as usize).min(max_scroll);
            total_events.saturating_sub(max_visible).saturating_sub(effective_scroll)
        } else {
            0
        };

        let visible_events: Vec<ListItem> = events
            .iter()
            .skip(start_idx)
            .take(max_visible)
            .map(|event| {
                let time_str = event.timestamp.format("%H:%M:%S").to_string();
                let (icon, color) = match event.event_type {
                    EventType::Info => ("ℹ", Color::Blue),
                    EventType::Success => ("✓", Color::Green),
                    EventType::Warning => ("⚠", Color::Yellow),
                    EventType::Error => ("✗", Color::Red),
                    EventType::Roll => ("🎲", Color::Cyan),
                    EventType::Claim => ("💖", Color::Magenta),
                    EventType::Kakera => ("💎", Color::Yellow),
                    EventType::Wishlist => ("⭐", Color::Magenta),
                };

                ListItem::new(Line::from(vec![
                    Span::styled(format!(" {} ", time_str), Style::default().fg(Color::DarkGray)),
                    Span::styled(format!("{}  ", icon), Style::default().fg(color)),
                    Span::styled(&event.message, Style::default().fg(Color::White)),
                ]))
            })
            .collect();

        let activity_list = List::new(visible_events).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green))
                .title(" Activity Log ")
                .title_style(Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        );

        frame.render_widget(activity_list, area);
    }

    fn render_channel_feed(frame: &mut Frame, area: Rect, activities: &[ChannelActivity]) {
        let max_visible = (area.height.saturating_sub(2)) as usize;
        
        let visible_items: Vec<ListItem> = activities
            .iter()
            .rev()
            .take(max_visible)
            .map(|activity| {
                match activity {
                    ChannelActivity::Roll { character_name, kakera_value, is_wished, claimed } => {
                        let name_style = if *is_wished {
                            Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)
                        } else if *claimed {
                            Style::default().fg(Color::Green)
                        } else {
                            Style::default().fg(Color::White)
                        };

                        let indicator = if *is_wished {
                            "⭐"
                        } else if *claimed {
                            "💖"
                        } else {
                            "🎲"
                        };

                        let kakera_str = kakera_value
                            .map(|v| format!(" ({}ka)", v))
                            .unwrap_or_default();

                        ListItem::new(Line::from(vec![
                            Span::raw(" "),
                            Span::styled(indicator, Style::default().fg(Color::Cyan)),
                            Span::raw("  "),
                            Span::styled(character_name.clone(), name_style),
                            Span::styled(kakera_str, Style::default().fg(Color::Yellow)),
                        ]))
                    }
                    ChannelActivity::UserMessage { username, content } => {
                        ListItem::new(Line::from(vec![
                            Span::raw(" "),
                            Span::styled(username.clone(), Style::default().fg(Color::Cyan)),
                            Span::styled(": ", Style::default().fg(Color::DarkGray)),
                            Span::styled(content.clone(), Style::default().fg(Color::White)),
                        ]))
                    }
                    ChannelActivity::MudaeInfo { message } => {
                        ListItem::new(Line::from(vec![
                            Span::raw(" "),
                            Span::styled("ℹ", Style::default().fg(Color::Blue)),
                            Span::raw("  "),
                            Span::styled(message.clone(), Style::default().fg(Color::DarkGray)),
                        ]))
                    }
                }
            })
            .collect();

        let feed_list = List::new(visible_items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Magenta))
                .title(" Channel Feed ")
                .title_style(Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
        );

        frame.render_widget(feed_list, area);
    }

    fn render_settings(frame: &mut Frame, area: Rect, cursor: usize, config: &Config, message: &Option<(String, bool)>) {
        let items = SettingsItem::all();
        
        let mut list_items: Vec<ListItem> = items
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let is_selected = i == cursor;
                let prefix = if is_selected { "► " } else { "  " };
                let label_style = if is_selected {
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };

                let value: Span = if item.is_toggle() {
                    let enabled = match item {
                        SettingsItem::AutoRoll => config.auto_roll,
                        SettingsItem::AutoKakera => config.auto_react_kakera,
                        SettingsItem::AutoDaily => config.auto_daily,
                        SettingsItem::Wishlist => config.wishlist_enabled,
                        SettingsItem::FuzzyMatch => config.fuzzy_match,
                        _ => false,
                    };
                    Self::status_indicator(enabled)
                } else {
                    match item {
                        SettingsItem::Token => Span::styled("********", Style::default().fg(Color::DarkGray)),
                        SettingsItem::Channels => Span::styled("Press Enter to edit", Style::default().fg(Color::DarkGray)),
                        SettingsItem::RollCommands => Span::styled(config.roll_commands.join(", "), Style::default().fg(Color::Cyan)),
                        SettingsItem::Cooldown => Span::styled(format!("{}s", config.roll_cooldown_seconds), Style::default().fg(Color::Cyan)),
                        _ => Span::raw(""),
                    }
                };

                ListItem::new(Line::from(vec![
                    Span::styled(prefix, label_style),
                    Span::styled(format!("{:<22}", item.label()), label_style),
                    value,
                ]))
            })
            .collect();

        if let Some((msg, success)) = message {
            list_items.push(ListItem::new(Line::from("")));
            let color = if *success { Color::Green } else { Color::Red };
            list_items.push(ListItem::new(Line::from(Span::styled(
                format!("  {}", msg),
                Style::default().fg(color),
            ))));
        }

        list_items.push(ListItem::new(Line::from("")));
        list_items.push(ListItem::new(Line::from(Span::styled(
            "  ↑↓ Navigate  •  Enter/Space Toggle  •  Esc Close",
            Style::default().fg(Color::DarkGray),
        ))));

        let list = List::new(list_items).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Yellow))
                .title(" Settings ")
                .title_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        );

        frame.render_widget(list, area);
    }

    fn render_text_input(
        frame: &mut Frame,
        area: Rect,
        title: &str,
        prompt: &str,
        input: &str,
        masked: bool,
        cursor: bool,
        message: &Option<(String, bool)>,
    ) {
        let cursor_char = if cursor { "▌" } else { " " };
        let display = if masked && !input.is_empty() {
            "*".repeat(input.len().min(40))
        } else {
            input.to_string()
        };

        let mut text = vec![
            Line::from(""),
            Line::from(Span::styled(format!("  {}", prompt), Style::default().fg(Color::White))),
            Line::from(""),
            Line::from(vec![
                Span::styled("  > ", Style::default().fg(Color::Yellow)),
                Span::styled(&display, Style::default().fg(Color::White)),
                Span::styled(cursor_char, Style::default().fg(Color::Yellow)),
            ]),
            Line::from(""),
        ];

        if let Some((msg, success)) = message {
            let color = if *success { Color::Green } else { Color::Red };
            text.push(Line::from(Span::styled(format!("  {}", msg), Style::default().fg(color))));
        }

        text.push(Line::from(""));
        text.push(Line::from(Span::styled("  Enter=save  •  Esc=cancel", Style::default().fg(Color::DarkGray))));

        let paragraph = Paragraph::new(text).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Cyan))
                .title(format!(" {} ", title))
                .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        );

        frame.render_widget(paragraph, area);
    }

    fn render_wishlist(
        frame: &mut Frame,
        area: Rect,
        characters: &[WishedCharacter],
        cursor: usize,
        message: &Option<(String, bool)>,
    ) {
        let inner = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(5),
                Constraint::Length(3),
            ])
            .margin(1)
            .split(area);

        let title_block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Magenta))
            .title(" ♥ Wishlist Manager ")
            .title_style(Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD));
        frame.render_widget(title_block, area);

        let header = Paragraph::new(Line::from(vec![
            Span::styled(format!(" {} characters ", characters.len()), Style::default().fg(Color::Cyan)),
            Span::styled("│", Style::default().fg(Color::DarkGray)),
            Span::styled(" A=Add  D=Delete  Esc=Back ", Style::default().fg(Color::DarkGray)),
        ]));
        frame.render_widget(header, inner[0]);

        if characters.is_empty() {
            let empty = Paragraph::new(vec![
                Line::from(""),
                Line::from(Span::styled("  No characters in wishlist", Style::default().fg(Color::DarkGray))),
                Line::from(""),
                Line::from(Span::styled("  Press 'A' to add a character", Style::default().fg(Color::Yellow))),
            ]);
            frame.render_widget(empty, inner[1]);
        } else {
            let visible_height = inner[1].height.saturating_sub(2) as usize;
            let start = cursor.saturating_sub(visible_height.saturating_sub(1));
            let end = (start + visible_height).min(characters.len());
            
            let list_items: Vec<ListItem> = characters[start..end]
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let actual_i = start + i;
                    let is_selected = actual_i == cursor;
                    let prefix = if is_selected { "► " } else { "  " };
                    
                    let verify_icon = if c.verified {
                        Span::styled("✓  ", Style::default().fg(Color::Green))
                    } else {
                        Span::styled("?  ", Style::default().fg(Color::Yellow))
                    };

                    let name_style = if is_selected {
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::White)
                    };

                    let series_display = c.series.as_ref()
                        .map(|s| format!(" ({})", s))
                        .unwrap_or_default();

                    let priority_display = if c.priority > 0 {
                        format!(" [P{}]", c.priority)
                    } else {
                        String::new()
                    };

                    ListItem::new(Line::from(vec![
                        Span::styled(prefix, name_style),
                        verify_icon,
                        Span::styled(&c.name, name_style),
                        Span::styled(series_display, Style::default().fg(Color::DarkGray)),
                        Span::styled(priority_display, Style::default().fg(Color::Cyan)),
                    ]))
                })
                .collect();

            let list = List::new(list_items);
            frame.render_widget(list, inner[1]);
        }

        let mut footer_text = vec![
            Span::styled(" ↑↓=Navigate  ", Style::default().fg(Color::DarkGray)),
        ];
        
        if let Some((msg, success)) = message {
            let color = if *success { Color::Green } else { Color::Red };
            footer_text.push(Span::styled(msg.clone(), Style::default().fg(color)));
        }

        let footer = Paragraph::new(Line::from(footer_text));
        frame.render_widget(footer, inner[2]);
    }

    fn render_search_character(
        frame: &mut Frame,
        area: Rect,
        input: &str,
        searching: bool,
        cursor: bool,
        message: &Option<(String, bool)>,
    ) {
        let cursor_char = if cursor && !searching { "▌" } else { " " };

        let mut text = vec![
            Line::from(""),
            Line::from(Span::styled("  🔍 Search & Add Character", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(Span::styled("  Enter character name to search:", Style::default().fg(Color::White))),
            Line::from(""),
            Line::from(vec![
                Span::styled("  > ", Style::default().fg(Color::Yellow)),
                Span::styled(input, Style::default().fg(Color::White)),
                Span::styled(cursor_char, Style::default().fg(Color::Yellow)),
            ]),
            Line::from(""),
        ];

        if searching {
            text.push(Line::from(Span::styled("  ◐  Searching...", Style::default().fg(Color::Yellow).add_modifier(Modifier::SLOW_BLINK))));
        }

        if let Some((msg, success)) = message {
            let color = if *success { Color::Green } else { Color::Red };
            text.push(Line::from(Span::styled(format!("  {}", msg), Style::default().fg(color))));
        }

        text.push(Line::from(""));
        text.push(Line::from(Span::styled("  Enter=Search  •  Esc=Cancel", Style::default().fg(Color::DarkGray))));

        let paragraph = Paragraph::new(text).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Magenta))
                .title(" Add Character ")
                .title_style(Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD)),
        );

        frame.render_widget(paragraph, area);
    }

    fn render_confirm_character(
        frame: &mut Frame,
        area: Rect,
        result: &SearchResult,
        message: &Option<(String, bool)>,
    ) {
        let mut text = vec![
            Line::from(""),
            Line::from(Span::styled("  ✓  Character Found!", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))),
            Line::from(""),
            Line::from(vec![
                Span::styled("  Name:   ", Style::default().fg(Color::DarkGray)),
                Span::styled(&result.name, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(vec![
                Span::styled("  Series: ", Style::default().fg(Color::DarkGray)),
                Span::styled(&result.series, Style::default().fg(Color::Cyan)),
            ]),
        ];

        if let Some(kakera) = result.kakera_value {
            text.push(Line::from(vec![
                Span::styled("  Kakera: ", Style::default().fg(Color::DarkGray)),
                Span::styled(format!("{}", kakera), Style::default().fg(Color::Yellow)),
            ]));
        }

        text.push(Line::from(""));
        text.push(Line::from(Span::styled("  ─────────────────────────────", Style::default().fg(Color::DarkGray))));
        text.push(Line::from(""));
        text.push(Line::from(Span::styled("  Add this character to your wishlist?", Style::default().fg(Color::White))));
        text.push(Line::from(""));

        if let Some((msg, success)) = message {
            let color = if *success { Color::Green } else { Color::Red };
            text.push(Line::from(Span::styled(format!("  {}", msg), Style::default().fg(color))));
            text.push(Line::from(""));
        }

        text.push(Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled("[Y]", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::styled(" Confirm   ", Style::default().fg(Color::DarkGray)),
            Span::styled("[N]", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            Span::styled(" Cancel", Style::default().fg(Color::DarkGray)),
        ]));

        let paragraph = Paragraph::new(text).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green))
                .title(" Confirm Character ")
                .title_style(Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        );

        frame.render_widget(paragraph, area);
    }

    fn status_indicator(enabled: bool) -> Span<'static> {
        if enabled {
            Span::styled("✓  Enabled", Style::default().fg(Color::Green))
        } else {
            Span::styled("✗  Disabled", Style::default().fg(Color::Red))
        }
    }

    async fn fetch_channel_names(
        client: Arc<crate::client::DiscordClient>,
        db: Arc<Database>,
        channel_ids: Vec<u64>,
    ) {
        for channel_id in channel_ids {
            if let Ok(channel) = client.get_channel(channel_id).await {
                let guild_name = if let Some(guild_id_str) = &channel.guild_id {
                    if let Ok(guild_id) = guild_id_str.parse::<u64>() {
                        client.get_guild(guild_id).await.ok().map(|g| g.name)
                    } else {
                        None
                    }
                } else {
                    None
                };
                
                if let Err(e) = db.update_channel_name(
                    channel_id,
                    channel.name.as_deref().unwrap_or("Unknown"),
                    guild_name.as_deref(),
                ) {
                    tracing::error!("Failed to update channel name: {}", e);
                }
            }
        }
    }

    pub fn cleanup(&mut self) -> Result<()> {
        disable_raw_mode()?;
        execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        self.terminal.show_cursor()?;
        Ok(())
    }
}

impl Drop for Tui {
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

pub async fn run_tui(
    stats: Arc<Stats>,
    config: Config,
    db: Arc<Database>,
    wishlist: Arc<WishlistManager>,
    search_tx: SearchRequestSender,
    shutdown_rx: watch::Receiver<bool>,
    channel_infos: Vec<ChannelInfo>,
    client: Option<crate::client::DiscordClient>,
) -> Result<()> {
    let client_arc = client.map(Arc::new);
    let mut tui = Tui::new(stats, config, db, wishlist, search_tx, shutdown_rx, channel_infos, client_arc)?;
    tui.run().await?;
    tui.cleanup()?;
    Ok(())
}
