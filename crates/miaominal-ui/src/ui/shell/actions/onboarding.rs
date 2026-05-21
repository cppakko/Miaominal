use super::super::*;

impl AppView {
    pub(in crate::ui::shell) fn open_onboarding(&mut self, cx: &mut Context<Self>) {
        self.onboarding.show_onboarding = true;
        self.onboarding.onboarding_step = OnboardingStep::Welcome;
        self.onboarding.visible_onboarding_step = OnboardingStep::Welcome;
        self.onboarding.onboarding_step_transition = None;
        cx.notify();
    }

    pub(in crate::ui::shell) fn finish_onboarding(&mut self, cx: &mut Context<Self>) {
        self.onboarding.show_onboarding = false;
        self.onboarding.onboarding_step = OnboardingStep::Welcome;
        self.onboarding.visible_onboarding_step = OnboardingStep::Welcome;
        self.onboarding.onboarding_step_transition = None;
        self.settings_store
            .update(|settings| settings.mark_current_onboarding_completed());
        cx.notify();
    }

    pub(in crate::ui::shell) fn advance_onboarding_step(&mut self, cx: &mut Context<Self>) {
        if let Some(next_step) = self.onboarding.onboarding_step.next() {
            self.onboarding.onboarding_step = next_step;
            cx.notify();
        }
    }

    pub(in crate::ui::shell) fn set_onboarding_step(
        &mut self,
        step: OnboardingStep,
        cx: &mut Context<Self>,
    ) {
        if self.onboarding.onboarding_step != step {
            self.onboarding.onboarding_step = step;
            cx.notify();
        }
    }
}
