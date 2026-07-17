use super::*;

pub(in crate::ui::shell) struct AppViewSubscriptionsArgs {
    pub rename_subscription: Subscription,
    pub keystroke_interceptor: Subscription,
}

pub(in crate::ui::shell) fn build_subscriptions(
    args: AppViewSubscriptionsArgs,
) -> AppViewSubscriptions {
    let AppViewSubscriptionsArgs {
        rename_subscription,
        keystroke_interceptor,
    } = args;

    AppViewSubscriptions {
        _rename_input_subscription: rename_subscription,
        _terminal_keystroke_interceptor: keystroke_interceptor,
    }
}
