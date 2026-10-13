use std::cell::RefCell;

use miaominal_core::{profile::SessionProfile, proxy::ProxyProfile};

use crate::ui::contracts::CatalogReader;

struct ConnectionCatalogSnapshot {
    profiles: Vec<SessionProfile>,
    proxies: Vec<ProxyProfile>,
}

/// ApplicationState adopts catalog changes; features only retain reader ports.
pub(super) struct ConnectionCatalog {
    snapshot: RefCell<ConnectionCatalogSnapshot>,
}

impl ConnectionCatalog {
    pub(super) fn new(profiles: Vec<SessionProfile>, proxies: Vec<ProxyProfile>) -> Self {
        Self {
            snapshot: RefCell::new(ConnectionCatalogSnapshot { profiles, proxies }),
        }
    }

    pub(super) fn replace(&self, source: &Self) {
        self.set_catalogs(source.profiles(), source.proxies());
    }

    pub(super) fn set_catalogs(
        &self,
        profiles: Vec<SessionProfile>,
        proxies: Vec<ProxyProfile>,
    ) -> bool {
        let mut snapshot = self.snapshot.borrow_mut();
        if snapshot.profiles == profiles && snapshot.proxies == proxies {
            return false;
        }
        *snapshot = ConnectionCatalogSnapshot { profiles, proxies };
        true
    }

    pub(super) fn set_profiles(&self, profiles: Vec<SessionProfile>) -> bool {
        let mut snapshot = self.snapshot.borrow_mut();
        if snapshot.profiles == profiles {
            return false;
        }
        snapshot.profiles = profiles;
        true
    }

    pub(super) fn set_proxies(&self, proxies: Vec<ProxyProfile>) -> bool {
        let mut snapshot = self.snapshot.borrow_mut();
        if snapshot.proxies == proxies {
            return false;
        }
        snapshot.proxies = proxies;
        true
    }

    pub(super) fn profiles(&self) -> Vec<SessionProfile> {
        self.snapshot.borrow().profiles.clone()
    }

    pub(super) fn proxies(&self) -> Vec<ProxyProfile> {
        self.snapshot.borrow().proxies.clone()
    }
}

impl CatalogReader for ConnectionCatalog {
    fn profiles(&self) -> Vec<SessionProfile> {
        self.profiles()
    }

    fn profile(&self, id: &str) -> Option<SessionProfile> {
        self.snapshot
            .borrow()
            .profiles
            .iter()
            .find(|profile| profile.id == id)
            .cloned()
    }

    fn proxies(&self) -> Vec<ProxyProfile> {
        self.proxies()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::contracts::CatalogQueryPort;
    use std::rc::Rc;

    #[test]
    fn retained_readers_follow_reloads_of_both_catalogs() {
        let catalog = Rc::new(ConnectionCatalog::new(
            vec![SessionProfile::blank("old", 1)],
            vec![ProxyProfile::blank("old-proxy", 1)],
        ));
        let reader = CatalogQueryPort::new(catalog.clone());
        let another_window = reader.clone();
        catalog.replace(&ConnectionCatalog::new(
            vec![SessionProfile::blank_local("new-local", 2)],
            vec![ProxyProfile::blank("new-proxy", 2)],
        ));

        assert!(reader.profile("old").is_none());
        assert_eq!(another_window.profiles()[0].id, "new-local");
        assert!(another_window.profile("new-local").unwrap().is_local());
        assert_eq!(reader.proxies()[0].id, "new-proxy");
        assert_eq!(another_window.proxies(), reader.proxies());
    }

    #[test]
    fn edited_snapshots_cannot_change_profiles_or_proxies() {
        let catalog = Rc::new(ConnectionCatalog::new(
            vec![SessionProfile::blank("saved", 1)],
            vec![ProxyProfile::blank("saved-proxy", 1)],
        ));
        let reader = CatalogQueryPort::new(catalog);
        let saved_profile = reader.profile("saved").unwrap();
        let saved_proxy = reader.proxies()[0].clone();
        let mut profiles = reader.profiles();
        profiles[0].name = "unsaved".to_string();
        profiles.clear();
        let mut individual = reader.profile("saved").unwrap();
        individual.host = "unsaved.example".to_string();
        let mut proxies = reader.proxies();
        proxies[0].host = "unsaved-proxy.example".to_string();
        proxies.clear();

        assert_eq!(reader.profiles(), vec![saved_profile]);
        assert_eq!(reader.proxies(), vec![saved_proxy]);
    }

    #[test]
    fn unchanged_adoption_is_not_a_catalog_change() {
        let profile = SessionProfile::blank("saved", 1);
        let proxy = ProxyProfile::blank("proxy", 1);
        let catalog = ConnectionCatalog::new(vec![profile.clone()], vec![proxy.clone()]);

        assert!(!catalog.set_catalogs(vec![profile.clone()], vec![proxy.clone()]));
        assert!(!catalog.set_profiles(vec![profile]));
        assert!(!catalog.set_proxies(vec![proxy]));
        assert!(catalog.set_profiles(vec![SessionProfile::blank_local("local", 1)]));
    }
}
