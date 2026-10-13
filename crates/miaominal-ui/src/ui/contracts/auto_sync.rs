use gpui_kit::App;
use std::rc::Rc;

type RequestCheck = dyn Fn(&App, bool, bool) -> bool;
type CancelCheck = dyn Fn(&App);

#[derive(Clone)]
pub(in crate::ui) struct AutoSyncControlPort {
    request: Rc<RequestCheck>,
    cancel: Rc<CancelCheck>,
}

impl AutoSyncControlPort {
    pub(in crate::ui) fn new(
        request: impl Fn(&App, bool, bool) -> bool + 'static,
        cancel: impl Fn(&App) + 'static,
    ) -> Self {
        Self {
            request: Rc::new(request),
            cancel: Rc::new(cancel),
        }
    }

    pub(in crate::ui) fn request_check(
        &self,
        cx: &App,
        enable: bool,
        unsafe_write_consent: bool,
    ) -> bool {
        (self.request)(cx, enable, unsafe_write_consent)
    }

    pub(in crate::ui) fn cancel_check(&self, cx: &App) {
        (self.cancel)(cx);
    }
}

impl std::fmt::Debug for AutoSyncControlPort {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AutoSyncControlPort")
            .finish_non_exhaustive()
    }
}
