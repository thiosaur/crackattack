//! Embedded distribution profile: the authority for the update and review system.
//! Always compiled, with or without the `update-check` feature. The contribute link, the
//! review destinations and the "managed by your store/distribution" note are static. Only
//! the remote part (HTTP client, reading the release) sits behind the feature (module
//! `update`).
//!
//! Chain of authority (each link can only restrict):
//! feature (build), public identity, profile (channel), remote release, local state.

/// Public identity of the project: the single source of every project destination.
/// With `None`, the GitHub surfaces and the release check disappear (no placeholder is shown,
/// no dead toggle).
pub const PUBLIC_PROJECT: Option<ProjectUrls> = Some(ProjectUrls {
    repo: "https://github.com/thiosaur/crackattack",
    releases: "https://github.com/thiosaur/crackattack/releases",
    latest_release_api: "https://api.github.com/repos/thiosaur/crackattack/releases/latest",
});

/// Store identifiers. Same `Option` logic as [`PUBLIC_PROJECT`]: they are filled in when
/// the matching package is submitted. Until then the review surfaces stay absent.
pub const STORE_IDS: StoreIds = StoreIds { microsoft: None, mac_app_store: None };

#[derive(Clone, Copy, Debug)]
pub struct ProjectUrls {
    /// Repository page ("Contribute" link, update fallback of the direct channel).
    pub repo: &'static str,
    /// Releases page (update destination of the direct channel).
    pub releases: &'static str,
    /// GitHub "latest release" API: `https://api.github.com/repos/{o}/{r}/releases/latest`.
    /// Only the checker reads it (hence the allow in builds without the feature). The type
    /// itself is always compiled.
    #[cfg_attr(not(feature = "update-check"), allow(dead_code))]
    pub latest_release_api: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct StoreIds {
    /// Microsoft Store ProductId (for example "9NBLGGH4NNS1").
    pub microsoft: Option<&'static str>,
    /// Numeric App Store identifier (the part after "id" in the URL).
    pub mac_app_store: Option<&'static str>,
}

/// Installation origin. The packaging pipeline stamps it (build variable
/// `CRACKATTACK_DISTRIBUTION`); it is never guessed. A missing or unknown stamp gives
/// [`Unknown`], which is fully closed (no network, no update, no review). Runtime
/// heuristics ($SNAP, $SteamAppId...) are for consistency diagnostics only, never for
/// permissions.
///
/// [`Unknown`]: Distribution::Unknown
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Distribution {
    Direct,
    MicrosoftStore,
    MacAppStore,
    Steam,
    Snap,
    Flathub,
    UbuntuRepository,
    Unknown,
}

impl Distribution {
    /// The stamp of this binary (set at compile time by the pipeline).
    pub fn current() -> Distribution {
        Self::from_stamp(option_env!("CRACKATTACK_DISTRIBUTION"))
    }

    /// Stamp value of this origin (`unknown` for [`Distribution::Unknown`]). Inverse of
    /// `from_stamp`; `--version` prints it.
    pub fn stamp(self) -> &'static str {
        match self {
            Distribution::Direct => "direct",
            Distribution::MicrosoftStore => "microsoft-store",
            Distribution::MacAppStore => "mac-app-store",
            Distribution::Steam => "steam",
            Distribution::Snap => "snap",
            Distribution::Flathub => "flathub",
            Distribution::UbuntuRepository => "ubuntu-repository",
            Distribution::Unknown => "unknown",
        }
    }

    fn from_stamp(stamp: Option<&str>) -> Distribution {
        match stamp {
            Some("direct") => Distribution::Direct,
            Some("microsoft-store") => Distribution::MicrosoftStore,
            Some("mac-app-store") => Distribution::MacAppStore,
            Some("steam") => Distribution::Steam,
            Some("snap") => Distribution::Snap,
            Some("flathub") => Distribution::Flathub,
            Some("ubuntu-repository") => Distribution::UbuntuRepository,
            // An unknown value means a misconfigured pipeline: closed, like a missing stamp.
            Some(_) | None => Distribution::Unknown,
        }
    }

    pub fn profile(self) -> &'static DistributionProfile {
        match self {
            Distribution::Direct => &DIRECT,
            Distribution::MicrosoftStore => &MICROSOFT_STORE,
            Distribution::MacAppStore => &MAC_APP_STORE,
            Distribution::Steam => &STEAM,
            Distribution::Snap => &SNAP,
            Distribution::Flathub => &FLATHUB,
            Distribution::UbuntuRepository => &UBUNTU_REPOSITORY,
            Distribution::Unknown => &UNKNOWN,
        }
    }
}

/// Right to read the remote release. It cannot come from the remote document itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FetchPolicy {
    Off,
    /// At most one attempt per 24 h, plus a manual button. Can be turned off (Options checkbox).
    AutoDaily,
}

/// What the channel does with the remote version. The fallback lives inside `Notify`, so
/// it cannot exist without a primary destination (the type enforces this).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UpdateUse {
    /// The channel's manager owns updates (stores, APT).
    Ignore,
    Notify { destination: Destination, fallback: Option<Destination> },
}

/// Review surface of a channel: a permanent button, never a prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReviewChannel {
    pub destination: Destination,
    pub fallback: Option<Destination>,
}

/// Note on the Info page: who manages updates when the game does not.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManagedBy {
    Store,
    Distro,
}

/// An openable destination: a closed identifier, resolved to a URL by [`resolve`] against
/// the binary's constants. The remote document never supplies an openable URL.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    ProjectReleases,
    ProjectRepo,
    MsStoreReview,
    MsStoreProductPage,
    MacAppStoreReview,
    MacAppStorePage,
}

/// Ecosystem of a destination. A fallback must stay in the ecosystem of its primary
/// (tested on all 8 profiles). Only tests use it, hence `cfg(test)`.
#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ecosystem {
    Project,
    MicrosoftStore,
    AppleStore,
}

impl Destination {
    #[cfg(test)]
    pub fn ecosystem(self) -> Ecosystem {
        match self {
            Destination::ProjectReleases | Destination::ProjectRepo => Ecosystem::Project,
            Destination::MsStoreReview | Destination::MsStoreProductPage => Ecosystem::MicrosoftStore,
            Destination::MacAppStoreReview | Destination::MacAppStorePage => Ecosystem::AppleStore,
        }
    }

    /// Resolves to an openable URL. Returns `None` if the needed identity or id is not set
    /// (the matching surface then disappears). It takes the values as parameters instead
    /// of reading the constants, so tests can exercise both states.
    pub fn resolve(self, project: Option<&ProjectUrls>, stores: &StoreIds) -> Option<String> {
        match self {
            Destination::ProjectReleases => project.map(|p| p.releases.to_string()),
            Destination::ProjectRepo => project.map(|p| p.repo.to_string()),
            Destination::MsStoreReview => stores
                .microsoft
                .map(|id| format!("ms-windows-store://review/?ProductId={id}")),
            Destination::MsStoreProductPage => stores
                .microsoft
                .map(|id| format!("https://apps.microsoft.com/detail/{id}")),
            // Form documented by Apple (not macappstore://).
            Destination::MacAppStoreReview => stores
                .mac_app_store
                .map(|id| format!("https://apps.apple.com/app/id{id}?action=write-review")),
            Destination::MacAppStorePage => stores
                .mac_app_store
                .map(|id| format!("https://apps.apple.com/app/id{id}")),
        }
    }
}

/// Capability grid of a channel. Fields are private and only this module's constants
/// build it. Cross-field invariants (`Notify` requires fetch enabled, a fallback stays in
/// the same ecosystem, `Unknown` has no network, no update and no review) cannot be
/// expressed by independent fields. The test `profils_invariants` checks them by
/// iterating over all distributions.
#[derive(Debug)]
pub struct DistributionProfile {
    // `fetch`: read by the checker (feature) and by tests, like `latest_release_api` above.
    #[cfg_attr(not(any(test, feature = "update-check")), allow(dead_code))]
    fetch: FetchPolicy,
    update: UpdateUse,
    // `review`: only the invariant tests read it for now. The Info page will read it once a
    // store package exists.
    #[cfg_attr(not(test), allow(dead_code))]
    review: Option<ReviewChannel>,
    contribute: Option<Destination>,
    managed_info: Option<ManagedBy>,
}

impl DistributionProfile {
    #[cfg_attr(not(any(test, feature = "update-check")), allow(dead_code))]
    pub fn fetch(&self) -> FetchPolicy {
        self.fetch
    }
    #[cfg(test)]
    pub fn update(&self) -> UpdateUse {
        self.update
    }
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn review(&self) -> Option<&ReviewChannel> {
        self.review.as_ref()
    }
    pub fn managed_info(&self) -> Option<ManagedBy> {
        self.managed_info
    }

    /// URL of the "Contribute" link. Static, independent of the `update-check` feature.
    /// `None` if the channel forbids it or the public identity does not exist yet.
    pub fn contribute_url(&self, project: Option<&ProjectUrls>) -> Option<String> {
        self.contribute?.resolve(project, &STORE_IDS)
    }

    /// URL of the update notification (the primary one). The fallback is only used if
    /// opening the primary fails.
    #[cfg_attr(not(any(test, feature = "update-check")), allow(dead_code))]
    pub fn update_url(&self, project: Option<&ProjectUrls>) -> Option<String> {
        match self.update {
            UpdateUse::Ignore => None,
            UpdateUse::Notify { destination, .. } => destination.resolve(project, &STORE_IDS),
        }
    }
}

const DIRECT: DistributionProfile = DistributionProfile {
    fetch: FetchPolicy::AutoDaily,
    update: UpdateUse::Notify {
        destination: Destination::ProjectReleases,
        fallback: Some(Destination::ProjectRepo),
    },
    review: None,
    contribute: Some(Destination::ProjectRepo),
    managed_info: None,
};

const MICROSOFT_STORE: DistributionProfile = DistributionProfile {
    fetch: FetchPolicy::Off,
    update: UpdateUse::Ignore, // the store notifies
    review: Some(ReviewChannel {
        destination: Destination::MsStoreReview,
        fallback: Some(Destination::MsStoreProductPage),
    }),
    contribute: Some(Destination::ProjectRepo),
    managed_info: Some(ManagedBy::Store),
};

const MAC_APP_STORE: DistributionProfile = DistributionProfile {
    fetch: FetchPolicy::Off,
    update: UpdateUse::Ignore, // App Store rule 2.4.5 (vii): updates go through the App Store
    review: Some(ReviewChannel {
        destination: Destination::MacAppStoreReview,
        fallback: Some(Destination::MacAppStorePage),
    }),
    contribute: None, // cautious: it could steer users outside the store; can be revisited
    managed_info: Some(ManagedBy::Store),
};

const STEAM: DistributionProfile = DistributionProfile {
    fetch: FetchPolicy::Off,
    update: UpdateUse::Ignore, // always up to date by construction
    review: None, // Steamworks: no review solicitation in game, not even a button
    contribute: Some(Destination::ProjectRepo), // external links are allowed; contributing is not reviewing
    managed_info: Some(ManagedBy::Store),
};

const SNAP: DistributionProfile = DistributionProfile {
    fetch: FetchPolicy::Off,
    update: UpdateUse::Ignore,
    review: None, // no verified mechanism
    contribute: Some(Destination::ProjectRepo),
    managed_info: Some(ManagedBy::Store),
};

const FLATHUB: DistributionProfile = DistributionProfile {
    fetch: FetchPolicy::Off,
    update: UpdateUse::Ignore,
    review: None,
    contribute: Some(Destination::ProjectRepo),
    managed_info: Some(ManagedBy::Store),
};

const UBUNTU_REPOSITORY: DistributionProfile = DistributionProfile {
    fetch: FetchPolicy::Off, // opt-in culture (like popularity-contest)
    update: UpdateUse::Ignore, // APT owns updates
    review: None,
    contribute: Some(Destination::ProjectRepo), // explicitly wanted
    managed_info: Some(ManagedBy::Distro),
};

const UNKNOWN: DistributionProfile = DistributionProfile {
    fetch: FetchPolicy::Off,
    update: UpdateUse::Ignore,
    review: None,
    // Static link, sends nothing: a build from source keeps the path to the repository.
    // Only the Mac App Store forbids it, and that channel requires a stamp.
    contribute: Some(Destination::ProjectRepo),
    managed_info: None,
};

/// The 8 channels, used by the invariant tests.
#[cfg(test)]
pub const ALL_DISTRIBUTIONS: [Distribution; 8] = [
    Distribution::Direct,
    Distribution::MicrosoftStore,
    Distribution::MacAppStore,
    Distribution::Steam,
    Distribution::Snap,
    Distribution::Flathub,
    Distribution::UbuntuRepository,
    Distribution::Unknown,
];

#[cfg(test)]
mod tests {
    use super::*;

    const FAKE_PROJECT: ProjectUrls = ProjectUrls {
        repo: "https://example.test/repo",
        releases: "https://example.test/releases",
        latest_release_api: "https://example.test/api/releases/latest",
    };
    const FAKE_STORES: StoreIds =
        StoreIds { microsoft: Some("MSID123"), mac_app_store: Some("4242") };

    /// Cross-field invariants of the grid, checked here because the type cannot express
    /// them.
    #[test]
    fn profils_invariants() {
        for d in ALL_DISTRIBUTIONS {
            let p = d.profile();
            if let UpdateUse::Notify { destination, fallback } = p.update {
                assert_ne!(p.fetch, FetchPolicy::Off, "{d:?}: Notify requires a fetch");
                if let Some(f) = fallback {
                    assert_eq!(f.ecosystem(), destination.ecosystem(),
                        "{d:?}: update fallback outside the ecosystem");
                }
            }
            if let Some(r) = &p.review {
                if let Some(f) = r.fallback {
                    assert_eq!(f.ecosystem(), r.destination.ecosystem(),
                        "{d:?}: review fallback outside the ecosystem");
                }
            }
        }
    }

    #[test]
    fn unknown_pas_de_reseau_mais_contribuer() {
        let p = Distribution::Unknown.profile();
        assert_eq!(p.fetch(), FetchPolicy::Off);
        assert_eq!(p.update(), UpdateUse::Ignore);
        assert!(p.review().is_none());
        assert!(p.managed_info().is_none());
        assert!(p.update_url(Some(&FAKE_PROJECT)).is_none());
        // A build from source keeps "Contribute" (static link).
        assert_eq!(p.contribute_url(Some(&FAKE_PROJECT)).as_deref(), Some("https://example.test/repo"));
    }

    /// `stamp()` is the exact inverse of `from_stamp` (what `--version` prints is what the
    /// pipeline set).
    #[test]
    fn stamp_aller_retour() {
        for d in ALL_DISTRIBUTIONS {
            let back = Distribution::from_stamp(Some(d.stamp()));
            assert_eq!(back, d, "{d:?}");
        }
        assert_eq!(Distribution::from_stamp(None).stamp(), "unknown");
    }

    /// No public identity means no project URL resolves anywhere, on any channel.
    #[test]
    fn identite_absente_aucune_url_projet() {
        for d in ALL_DISTRIBUTIONS {
            let p = d.profile();
            assert!(p.contribute_url(None).is_none(), "{d:?}");
            assert!(p.update_url(None).is_none(), "{d:?}");
        }
        // The binary's real identity points to the public repository; no store ids yet.
        let project = PUBLIC_PROJECT.expect("public identity");
        assert!(project.releases.starts_with(project.repo));
        assert!(project.latest_release_api.ends_with("/repos/thiosaur/crackattack/releases/latest"));
        assert!(STORE_IDS.microsoft.is_none() && STORE_IDS.mac_app_store.is_none());
    }

    /// The Mac App Store profile never resolves a project or GitHub destination, even with
    /// identity and ids set (rule 2.4.5 vii, plus the cautious contribute setting).
    #[test]
    fn mac_app_store_jamais_le_projet() {
        let p = Distribution::MacAppStore.profile();
        assert!(p.update_url(Some(&FAKE_PROJECT)).is_none());
        assert!(p.contribute_url(Some(&FAKE_PROJECT)).is_none());
        let r = p.review().expect("MAS has a review channel");
        for dest in [Some(r.destination), r.fallback].into_iter().flatten() {
            let url = dest.resolve(Some(&FAKE_PROJECT), &FAKE_STORES).unwrap();
            assert!(url.starts_with("https://apps.apple.com/"), "{url}");
        }
    }

    #[test]
    fn steam_aucune_surface_avis_mais_contribution() {
        let p = Distribution::Steam.profile();
        assert!(p.review().is_none());
        assert_eq!(p.contribute_url(Some(&FAKE_PROJECT)).as_deref(), Some("https://example.test/repo"));
    }

    #[test]
    fn debian_contribution_survit_a_l_extinction() {
        let p = Distribution::UbuntuRepository.profile();
        assert_eq!(p.fetch(), FetchPolicy::Off);
        assert_eq!(p.update(), UpdateUse::Ignore);
        assert_eq!(p.contribute_url(Some(&FAKE_PROJECT)).as_deref(), Some("https://example.test/repo"));
        assert_eq!(p.managed_info(), Some(ManagedBy::Distro));
    }

    #[test]
    fn ms_store_retombe_seulement_sur_microsoft() {
        let r = Distribution::MicrosoftStore.profile().review().unwrap();
        let fb = r.fallback.unwrap().resolve(Some(&FAKE_PROJECT), &FAKE_STORES).unwrap();
        assert!(fb.starts_with("https://apps.microsoft.com/"), "{fb}");
    }

    #[test]
    fn apple_write_review_forme_documentee() {
        let url = Destination::MacAppStoreReview.resolve(None, &FAKE_STORES).unwrap();
        assert_eq!(url, "https://apps.apple.com/app/id4242?action=write-review");
    }

    #[test]
    fn estampille_absente_ou_inconnue_ferme() {
        assert_eq!(Distribution::from_stamp(None), Distribution::Unknown);
        assert_eq!(Distribution::from_stamp(Some("appstore-typo")), Distribution::Unknown);
        assert_eq!(Distribution::from_stamp(Some("direct")), Distribution::Direct);
        assert_eq!(Distribution::from_stamp(Some("steam")), Distribution::Steam);
    }
}
