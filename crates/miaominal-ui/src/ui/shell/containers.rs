use gpui::Subscription;
use std::ops::{Deref, DerefMut};

const ROOT_SHELL_SUBSCRIPTION_COUNT: usize = 2;
const ROOT_SUBSCRIPTION_LIMIT: usize = 12;

pub(in crate::ui::shell) struct AppViewSubscriptions {
    pub _rename_input_subscription: Subscription,
    pub _terminal_keystroke_interceptor: Subscription,
}

pub(in crate::ui::shell) struct RootSubscriptions {
    legacy: AppViewSubscriptions,
    _controllers: Vec<Subscription>,
}

impl RootSubscriptions {
    pub(in crate::ui::shell) fn new(
        legacy: AppViewSubscriptions,
        controllers: Vec<Subscription>,
    ) -> Self {
        debug_assert!(
            controllers.len() + ROOT_SHELL_SUBSCRIPTION_COUNT <= ROOT_SUBSCRIPTION_LIMIT,
            "root subscription limit exceeded"
        );
        Self {
            legacy,
            _controllers: controllers,
        }
    }
}

impl Deref for RootSubscriptions {
    type Target = AppViewSubscriptions;

    fn deref(&self) -> &Self::Target {
        &self.legacy
    }
}

impl DerefMut for RootSubscriptions {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.legacy
    }
}
