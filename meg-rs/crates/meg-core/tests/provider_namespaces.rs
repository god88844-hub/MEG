#[test]
fn alternate_configs_cannot_read_replace_or_delete_each_others_keys() {
    use meg_core::providers::{
        api_key, vault, ProviderProfile, ProviderSettings, ProviderStore, Secret,
    };
    struct Fixture {
        id: String,
        roots: Vec<std::path::PathBuf>,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = vault::remove(&self.id);
            for root in &self.roots {
                let _ = vault::remove_scoped(
                    &self.id,
                    meg_core::providers::credential_scope(root).as_deref(),
                );
                assert_eq!(root.parent().unwrap(), std::env::temp_dir());
                let _ = std::fs::remove_dir_all(root);
            }
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let roots: Vec<_> = (0..2)
        .map(|_| {
            std::env::temp_dir().join(format!("meg-provider-namespace-{}", uuid::Uuid::new_v4()))
        })
        .collect();
    for root in &roots {
        std::fs::create_dir(root).unwrap();
    }
    let fixture = Fixture {
        id: id.clone(),
        roots,
    };
    vault::set(&id, &Secret::new("legacy-generated-fixture".into())).unwrap();
    let mut profile = ProviderProfile::environment();
    profile.id = id.clone();
    profile.key_env = None;
    let first = ProviderStore {
        root: fixture.roots[0].clone(),
    };
    let key = Secret::new("isolated-generated-fixture".into());
    let saved = first.upsert(profile, Some(&key)).unwrap();
    let profile = saved.profiles.iter().find(|p| p.id == id).unwrap().clone();
    assert_eq!(
        api_key(&profile).unwrap().unwrap().expose(),
        "isolated-generated-fixture"
    );
    assert_eq!(
        vault::get(&id).unwrap().unwrap().expose(),
        "legacy-generated-fixture"
    );
    let second = ProviderStore {
        root: fixture.roots[1].clone(),
    };
    second
        .save(&ProviderSettings {
            version: 1,
            active: id.clone(),
            profiles: vec![profile.clone()],
        })
        .unwrap();
    let other = second.load().unwrap().profiles.remove(0);
    assert_ne!(other.credential_scope, profile.credential_scope);
    assert!(api_key(&other).unwrap().is_none());
    second.remove(&id).unwrap();
    assert_eq!(
        api_key(&profile).unwrap().unwrap().expose(),
        "isolated-generated-fixture"
    );
    first.remove(&id).unwrap();
    assert_eq!(
        vault::get(&id).unwrap().unwrap().expose(),
        "legacy-generated-fixture"
    );
}
