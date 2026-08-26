use crossterm::event::KeyEvent;
use ratatui::prelude::{Rect, Layout, Direction, Constraint, Widget};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, info};

use crate::{
    action::Action,
    components::{
        Component,
        sidebar::Sidebar,
        clients::Clients,
        mcps::Mcps,
        containers::Containers,
        skills::Skills,
        mcp_browser::McpBrowser,
        profile_switcher::ProfileSwitcher,
        profile_editor::ProfileEditor,
        wizard::Wizard,
        diff_viewer::DiffViewer,
        inspector::Inspector,
        server_browser::ServerBrowser,
        greeting::Greeting,
        help::Help,
        sync_confirm::SyncConfirm,
    },
    config::Config,
    state::AppState,
    tui::{Event, Tui},
};

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Clients,
    Mcps,
    Skills,
}

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActiveModal {
    #[default]
    None,
    Help,
    McpBrowser,
    ProfileSwitcher,
    ProfileEditor,
    Wizard,
    SyncConfirm,
    ServerBrowser,
    DiffViewer,
    Inspector,
}

pub struct App {
    config: Config,
    app_state: AppState,
    tick_rate: f64,
    frame_rate: f64,
    should_quit: bool,
    should_suspend: bool,
    mode: Mode,
    active_modal: ActiveModal,
    last_tick_key_events: Vec<KeyEvent>,
    action_tx: mpsc::UnboundedSender<Action>,
    action_rx: mpsc::UnboundedReceiver<Action>,

    sidebar: Sidebar,
    clients: Clients,
    mcps: Mcps,
    containers: Containers,
    skills: Skills,
    mcp_browser: McpBrowser,
    profile_switcher: ProfileSwitcher,
    profile_editor: ProfileEditor,
    wizard: Wizard,
    diff_viewer: DiffViewer,
    inspector: Inspector,
    server_browser: ServerBrowser,
    greeting: Greeting,
    help: Help,
    sync_confirm: SyncConfirm,
}

impl App {
    pub fn new(tick_rate: f64, frame_rate: f64) -> color_eyre::Result<Self> {
        let (action_tx, action_rx) = mpsc::unbounded_channel();
        let config_path = crate::state::get_config_path();
        
        let app_state = if let Ok(content) = std::fs::read_to_string(&config_path) {
            serde_json::from_str(&content).unwrap_or_else(|_| AppState {
                version: "1.0.0".into(),
                settings: crate::state::GlobalSettings {
                    active_profile: "default".into(),
                    docker_socket: None,
                    sync_clients: vec![],
                    filter_managed_containers_only: None,
                },
                servers: Default::default(),
                profiles: Default::default(),
                workspaces: Default::default(),
                docker_status: String::new(),
                containers: Vec::new(),
                container_logs: Vec::new(),
                inspector_events: Vec::new(),
            })
        } else {
            AppState {
                version: "1.0.0".into(),
                settings: crate::state::GlobalSettings {
                    active_profile: "default".into(),
                    docker_socket: None,
                    sync_clients: vec![],
                    filter_managed_containers_only: None,
                },
                servers: Default::default(),
                profiles: Default::default(),
                workspaces: Default::default(),
                docker_status: String::new(),
                containers: Vec::new(),
                container_logs: Vec::new(),
                inspector_events: Vec::new(),
            }
        };

        Ok(Self {
            tick_rate,
            frame_rate,
            should_quit: false,
            should_suspend: false,
            config: Config::new()?,
            app_state,
            mode: Mode::Clients,
            active_modal: ActiveModal::None,
            last_tick_key_events: Vec::new(),
            action_tx,
            action_rx,
            
            sidebar: Sidebar::default(),
            clients: Clients::default(),
            mcps: Mcps { selected_index: 0, bottom_mode: crate::components::mcps::McpBottomPaneMode::Logs, logs_scroll_offset: 0, inspector_selected_index: 0, inspector_scroll_offset: 0, filter_mode: crate::components::mcps::ContainerFilterMode::ManagedOnly, command_tx: None },
            containers: Containers::default(),
            skills: Skills::default(),
            mcp_browser: McpBrowser::default(),
            profile_switcher: ProfileSwitcher::default(),
            profile_editor: ProfileEditor::default(),
            wizard: crate::components::wizard::Wizard::default(),
            diff_viewer: DiffViewer::default(),
            inspector: Inspector::default(),
            server_browser: ServerBrowser::default(),
            greeting: Greeting::default(),
            help: Help::default(),
            sync_confirm: SyncConfirm::default(),
        })
    }

    pub async fn run(&mut self) -> color_eyre::Result<()> {
        let mut tui = Tui::new()?
            .tick_rate(self.tick_rate)
            .frame_rate(self.frame_rate);
        tui.enter()?;

        // Register tx
        self.sidebar.register_action_handler(self.action_tx.clone())?;
        self.clients.register_action_handler(self.action_tx.clone())?;
        self.mcps.register_action_handler(self.action_tx.clone())?;
        self.containers.register_action_handler(self.action_tx.clone())?;
        self.skills.register_action_handler(self.action_tx.clone())?;
        self.mcp_browser.register_action_handler(self.action_tx.clone())?;
        self.profile_switcher.register_action_handler(self.action_tx.clone())?;
        self.profile_editor.register_action_handler(self.action_tx.clone())?;
        self.wizard.register_action_handler(self.action_tx.clone())?;
        self.diff_viewer.register_action_handler(self.action_tx.clone())?;
        self.inspector.register_action_handler(self.action_tx.clone())?;
        self.server_browser.register_action_handler(self.action_tx.clone())?;
        self.greeting.register_action_handler(self.action_tx.clone())?;
        self.help.register_action_handler(self.action_tx.clone())?;
        self.sync_confirm.register_action_handler(self.action_tx.clone())?;

        let action_tx = self.action_tx.clone();
        loop {
            self.handle_events(&mut tui).await?;
            self.handle_actions(&mut tui)?;
            if self.should_suspend {
                tui.suspend()?;
                action_tx.send(Action::Resume)?;
                action_tx.send(Action::ClearScreen)?;
                tui.enter()?;
            } else if self.should_quit {
                tui.stop()?;
                break;
            }
        }
        tui.exit()?;
        Ok(())
    }

    async fn handle_events(&mut self, tui: &mut Tui) -> color_eyre::Result<()> {
        let Some(event) = tui.next_event().await else {
            return Ok(());
        };
        let action_tx = self.action_tx.clone();
        match event {
            Event::Quit => action_tx.send(Action::Quit)?,
            Event::Tick => action_tx.send(Action::Tick)?,
            Event::Render => action_tx.send(Action::Render)?,
            Event::Resize(x, y) => action_tx.send(Action::Resize(x, y))?,
            Event::Key(key) => self.handle_key_event(key)?,
            _ => {}
        }
        
        // Route event to modal or active view
        let mut handled = false;
        
        if !handled {
            if let Some(action) = self.sidebar.handle_events(Some(event.clone()))? {
                action_tx.send(action)?;
                handled = true;
            }
        }
        
        if !handled {
            match self.mode {
                Mode::Clients => {
                    if let Some(action) = self.clients.handle_events(Some(event.clone()))? {
                        action_tx.send(action)?;
                        handled = true;
                    }
                }
                Mode::Mcps => {
                    if let Some(action) = self.mcps.handle_events(Some(event.clone()))? {
                        action_tx.send(action)?;
                        handled = true;
                    }
                }
                Mode::Skills => {
                    if let Some(action) = self.skills.handle_events(Some(event.clone()))? {
                        action_tx.send(action)?;
                        handled = true;
                    }
                }
                _ => {}
            }
        }
        
        Ok(())
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> color_eyre::Result<()> {
        // Skip for now, rely on component handle_events
        Ok(())
    }

    fn handle_actions(&mut self, tui: &mut Tui) -> color_eyre::Result<()> {
        while let Ok(action) = self.action_rx.try_recv() {
            match action {
                Action::Tick => {
                    self.last_tick_key_events.drain(..);
                }
                Action::Quit => self.should_quit = true,
                Action::Suspend => self.should_suspend = true,
                Action::Resume => self.should_suspend = false,
                Action::ClearScreen => tui.terminal.clear()?,
                Action::Resize(w, h) => self.handle_resize(tui, w, h)?,
                Action::Render => self.render(tui)?,
                Action::Error(ref err) => tracing::error!(?err),
                _ => {}
            }
            
            // Pass to components
            self.sidebar.update(action.clone())?;
            self.clients.update(action.clone())?;
            self.mcps.update(action.clone())?;
            self.skills.update(action.clone())?;
            // Add other updates as needed
        }
        Ok(())
    }

    fn handle_resize(&mut self, tui: &mut Tui, w: u16, h: u16) -> color_eyre::Result<()> {
        tui.resize(Rect::new(0, 0, w, h))?;
        self.render(tui)?;
        Ok(())
    }

    fn render(&mut self, tui: &mut Tui) -> color_eyre::Result<()> {
        tui.draw(|frame| {
            let size = frame.area();

            let root_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(10), Constraint::Length(1)])
                .split(size);

            let main_split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(25), Constraint::Min(20)])
                .split(root_chunks[0]);

            frame.render_stateful_widget(&self.sidebar, main_split[0], &mut self.app_state);

            match self.mode {
                Mode::Clients => frame.render_stateful_widget(&self.clients, main_split[1], &mut self.app_state),
                Mode::Mcps => frame.render_stateful_widget(&self.mcps, main_split[1], &mut self.app_state),
                Mode::Skills => frame.render_widget(&self.skills, main_split[1]),
                _ => {}
            }
        })?;
        Ok(())
    }
}
