#![allow(dead_code)]

use gpui::{App, Entity, WeakEntity};

use super::AppView;

#[derive(Clone)]
struct ControllerHandle {
    app: WeakEntity<AppView>,
}

impl ControllerHandle {
    fn new(app: WeakEntity<AppView>) -> Self {
        Self { app }
    }

    fn upgrade(&self) -> Option<Entity<AppView>> {
        self.app.upgrade()
    }

    #[allow(dead_code)]
    fn with<R>(
        &self,
        cx: &mut App,
        f: impl FnOnce(&mut AppView, &mut gpui::Context<AppView>) -> R,
    ) -> Option<R> {
        self.upgrade().map(|app| app.update(cx, f))
    }
}

#[derive(Clone)]
pub struct ProfileController {
    handle: ControllerHandle,
}

impl ProfileController {
    pub fn new(app: WeakEntity<AppView>) -> Self {
        Self {
            handle: ControllerHandle::new(app),
        }
    }
}

#[derive(Clone)]
pub struct TerminalController {
    handle: ControllerHandle,
}

impl TerminalController {
    pub fn new(app: WeakEntity<AppView>) -> Self {
        Self {
            handle: ControllerHandle::new(app),
        }
    }
}

#[derive(Clone)]
pub struct SftpController {
    handle: ControllerHandle,
}

impl SftpController {
    pub fn new(app: WeakEntity<AppView>) -> Self {
        Self {
            handle: ControllerHandle::new(app),
        }
    }
}

#[derive(Clone)]
pub struct SyncController {
    handle: ControllerHandle,
}

impl SyncController {
    pub fn new(app: WeakEntity<AppView>) -> Self {
        Self {
            handle: ControllerHandle::new(app),
        }
    }
}

#[derive(Clone)]
pub struct SettingsController {
    handle: ControllerHandle,
}

impl SettingsController {
    pub fn new(app: WeakEntity<AppView>) -> Self {
        Self {
            handle: ControllerHandle::new(app),
        }
    }
}

#[derive(Clone)]
pub struct KeychainController {
    handle: ControllerHandle,
}

impl KeychainController {
    pub fn new(app: WeakEntity<AppView>) -> Self {
        Self {
            handle: ControllerHandle::new(app),
        }
    }
}

#[derive(Clone)]
pub struct ControllerSet {
    pub profile: ProfileController,
    pub terminal: TerminalController,
    pub sftp: SftpController,
    pub sync: SyncController,
    pub settings: SettingsController,
    pub keychain: KeychainController,
}

impl ControllerSet {
    pub fn new(app: WeakEntity<AppView>) -> Self {
        Self {
            profile: ProfileController::new(app.clone()),
            terminal: TerminalController::new(app.clone()),
            sftp: SftpController::new(app.clone()),
            sync: SyncController::new(app.clone()),
            settings: SettingsController::new(app.clone()),
            keychain: KeychainController::new(app),
        }
    }
}
