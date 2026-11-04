//! Autorisations de capture et verrou d'analyse (décision D6).
//!
//! ## Avant toute capture
//!
//! Une capture n'est possible qu'avec une [`CaptureAuthorization`], que seul
//! [`ConsentRegistry::authorize_capture`] délivre, et seulement si :
//!
//! 1. l'utilisateur a donné son consentement à l'enregistrement, sans l'avoir retiré ;
//! 2. il a accepté la **version en vigueur** des conditions de la salle ([`RoomTerms`]).
//!
//! L'autorisation n'est ni clonable ni désérialisable : on ne peut pas la fabriquer, et elle est
//! consommée par [`AnalysisLock::begin_capture`].
//!
//! ## Verrou d'analyse
//!
//! Pendant une capture, aucune information stratégique ne doit être produite (D6). Le
//! [`AnalysisLock`] l'impose par les types :
//!
//! - [`AnalysisLock::begin_capture`] renvoie un [`CaptureGuard`] ; tant qu'il existe, toute
//!   demande d'analyse ou d'entraînement est refusée ([`AnalysisDenied::CaptureInProgress`]) ;
//! - une analyse exige un [`AnalysisPermit`], délivré par [`AnalysisLock::authorize_analysis`]
//!   seulement si aucune capture n'est en cours **et** si la session est close
//!   ([`Session::is_open`]) ; les modules d'analyse prennent ce permis en paramètre ;
//! - inversement, une capture ne démarre pas tant qu'un permis d'analyse ou d'entraînement
//!   est vivant : les deux modes sont distincts.
//!
//! L'application doit partager **un seul** verrou ([`AnalysisLock::process_wide`]) entre la
//! capture et l'analyse.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};

use serde::{Deserialize, Serialize};
use stratyx_poker::session::{Session, SessionSource};

/// Durée de validité d'une autorisation de capture : la session doit commencer dans les
/// 10 minutes qui suivent sa délivrance.
pub const AUTHORIZATION_VALIDITY_MS: u64 = 10 * 60 * 1000;

/// Longueur maximale d'un identifiant de salle.
const MAX_ROOM_ID_LEN: usize = 64;

/// Erreur d'autorisation ou de démarrage de capture.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CaptureError {
    /// Identifiant de salle vide, trop long ou avec des caractères interdits.
    #[error("identifiant de salle invalide : « {0} »")]
    InvalidRoom(String),
    /// Aucun consentement à l'enregistrement.
    #[error("aucun consentement à l'enregistrement n'a été donné : capture impossible")]
    ConsentMissing,
    /// Le consentement a été retiré.
    #[error("le consentement à l'enregistrement a été retiré : capture impossible")]
    ConsentWithdrawn,
    /// Conditions de la salle jamais acceptées.
    #[error("les conditions de la salle « {room} » n'ont pas été acceptées : capture impossible")]
    TermsNotAccepted {
        /// Salle concernée.
        room: RoomId,
    },
    /// Conditions de la salle acceptées dans une version qui n'est plus en vigueur.
    #[error(
        "les conditions de la salle « {room} » ont changé (version acceptée : {accepted}, \
         en vigueur : {current}) : nouvelle acceptation requise"
    )]
    TermsOutdated {
        /// Salle concernée.
        room: RoomId,
        /// Version acceptée par l'utilisateur.
        accepted: String,
        /// Version en vigueur.
        current: String,
    },
    /// Seules les captures en direct passent par le verrou.
    #[error("la session « {session_id} » n'est pas une capture en direct")]
    NotLiveCapture {
        /// Session concernée.
        session_id: String,
    },
    /// La session est déjà close.
    #[error("la session « {session_id} » est déjà close")]
    SessionAlreadyClosed {
        /// Session concernée.
        session_id: String,
    },
    /// La session est déjà en cours de capture.
    #[error("la session « {session_id} » est déjà en cours de capture")]
    AlreadyCapturing {
        /// Session concernée.
        session_id: String,
    },
    /// La session a commencé avant la délivrance de l'autorisation.
    #[error("la session « {session_id} » a commencé avant l'autorisation de capture")]
    StartedBeforeAuthorization {
        /// Session concernée.
        session_id: String,
    },
    /// Autorisation trop ancienne.
    #[error("autorisation de capture périmée (plus de 10 minutes) : la redemander")]
    AuthorizationExpired,
    /// Une analyse ou un entraînement est en cours.
    #[error(
        "{permits} analyse(s) ou entraînement(s) en cours : quitter ce mode avant de lancer \
         une capture (décision D6)"
    )]
    AnalysisInProgress {
        /// Nombre de permis encore vivants.
        permits: usize,
    },
}

/// Refus d'une demande d'analyse stratégique ou d'entraînement (décision D6).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AnalysisDenied {
    /// Une capture est en cours : aucune information stratégique ne peut être produite.
    #[error(
        "analyse stratégique refusée : capture en cours (session {}) — aucune aide pendant \
         une partie réelle (décision D6)",
        .session_ids.join(", ")
    )]
    CaptureInProgress {
        /// Sessions en cours de capture.
        session_ids: Vec<String>,
    },
    /// La session à analyser n'est pas close.
    #[error(
        "analyse stratégique refusée : la session « {session_id} » n'est pas close \
         (décision D6)"
    )]
    SessionOpen {
        /// Session concernée.
        session_id: String,
    },
}

/// Identifiant normalisé d'une salle de poker (minuscules ; lettres, chiffres, `-`, `_`, `.`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RoomId(String);

impl RoomId {
    /// Valide et normalise un identifiant de salle (espaces retirés, minuscules).
    ///
    /// # Errors
    /// [`CaptureError::InvalidRoom`].
    pub fn new(id: &str) -> Result<Self, CaptureError> {
        let normalized = id.trim().to_lowercase();
        let valid = !normalized.is_empty()
            && normalized.len() <= MAX_ROOM_ID_LEN
            && normalized
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        if valid {
            Ok(Self(normalized))
        } else {
            Err(CaptureError::InvalidRoom(id.to_owned()))
        }
    }

    /// Identifiant normalisé.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RoomId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for RoomId {
    type Error = CaptureError;

    fn try_from(id: String) -> Result<Self, Self::Error> {
        Self::new(&id)
    }
}

impl From<RoomId> for String {
    fn from(id: RoomId) -> Self {
        id.0
    }
}

/// Conditions d'utilisation **en vigueur** d'une salle, telles que vérifiées (version ou date
/// de publication). Ce qu'elles autorisent (enregistrement, outils d'analyse après partie,
/// statistiques sur les adversaires) est à vérifier salle par salle.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RoomTerms {
    /// Salle.
    pub room: RoomId,
    /// Version des conditions (ex. « 2026-09-01 »).
    pub version: String,
}

/// Consentement de l'utilisateur à l'enregistrement de ses parties.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentRecord {
    /// Version du texte de consentement présenté.
    pub statement_version: String,
    /// Date du consentement (ms depuis l'époque Unix, UTC).
    pub given_at_ms: u64,
    /// Date du retrait, le cas échéant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub withdrawn_at_ms: Option<u64>,
}

/// Acceptation des conditions d'une salle par l'utilisateur.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermsAcceptance {
    /// Salle.
    pub room: RoomId,
    /// Version acceptée.
    pub terms_version: String,
    /// Date d'acceptation (ms depuis l'époque Unix, UTC).
    pub accepted_at_ms: u64,
}

/// Registre persistant (sérialisable) des consentements et acceptations, conservé comme
/// preuve : rien n'en est effacé, un retrait s'ajoute à l'historique.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentRegistry {
    #[serde(default)]
    consents: Vec<ConsentRecord>,
    #[serde(default)]
    acceptances: Vec<TermsAcceptance>,
}

impl ConsentRegistry {
    /// Registre vide.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Enregistre le consentement de l'utilisateur au texte `statement_version`.
    pub fn record_consent(&mut self, statement_version: impl Into<String>, now_ms: u64) {
        self.consents.push(ConsentRecord {
            statement_version: statement_version.into(),
            given_at_ms: now_ms,
            withdrawn_at_ms: None,
        });
    }

    /// Retire le consentement en cours : toute nouvelle capture est refusée.
    ///
    /// # Errors
    /// [`CaptureError::ConsentMissing`] s'il n'y a pas de consentement actif.
    pub fn withdraw_consent(&mut self, now_ms: u64) -> Result<(), CaptureError> {
        match self.consents.last_mut() {
            Some(consent) if consent.withdrawn_at_ms.is_none() => {
                consent.withdrawn_at_ms = Some(now_ms);
                Ok(())
            }
            _ => Err(CaptureError::ConsentMissing),
        }
    }

    /// Enregistre l'acceptation des conditions `terms`.
    pub fn accept_terms(&mut self, terms: &RoomTerms, now_ms: u64) {
        self.acceptances.push(TermsAcceptance {
            room: terms.room.clone(),
            terms_version: terms.version.clone(),
            accepted_at_ms: now_ms,
        });
    }

    /// Consentement le plus récent, retiré ou non.
    #[must_use]
    pub fn latest_consent(&self) -> Option<&ConsentRecord> {
        self.consents.last()
    }

    /// Acceptation la plus récente des conditions d'une salle.
    #[must_use]
    pub fn latest_acceptance(&self, room: &RoomId) -> Option<&TermsAcceptance> {
        self.acceptances.iter().rev().find(|a| &a.room == room)
    }

    /// Historique complet des consentements.
    #[must_use]
    pub fn consents(&self) -> &[ConsentRecord] {
        &self.consents
    }

    /// Historique complet des acceptations.
    #[must_use]
    pub fn acceptances(&self) -> &[TermsAcceptance] {
        &self.acceptances
    }

    /// Délivre une autorisation de capture pour la salle de `terms` si le consentement est
    /// actif et la version en vigueur des conditions acceptée.
    ///
    /// # Errors
    /// [`CaptureError::ConsentMissing`], [`CaptureError::ConsentWithdrawn`],
    /// [`CaptureError::TermsNotAccepted`], [`CaptureError::TermsOutdated`].
    pub fn authorize_capture(
        &self,
        terms: &RoomTerms,
        now_ms: u64,
    ) -> Result<CaptureAuthorization, CaptureError> {
        let consent = self.latest_consent().ok_or(CaptureError::ConsentMissing)?;
        if consent.withdrawn_at_ms.is_some() {
            return Err(CaptureError::ConsentWithdrawn);
        }
        let acceptance =
            self.latest_acceptance(&terms.room)
                .ok_or_else(|| CaptureError::TermsNotAccepted {
                    room: terms.room.clone(),
                })?;
        if acceptance.terms_version != terms.version {
            return Err(CaptureError::TermsOutdated {
                room: terms.room.clone(),
                accepted: acceptance.terms_version.clone(),
                current: terms.version.clone(),
            });
        }
        Ok(CaptureAuthorization {
            room: terms.room.clone(),
            terms_version: terms.version.clone(),
            consent_given_at_ms: consent.given_at_ms,
            terms_accepted_at_ms: acceptance.accepted_at_ms,
            granted_at_ms: now_ms,
        })
    }
}

/// Preuve, non falsifiable par le code appelant, que la capture a été autorisée.
///
/// Ni `Clone`, ni `Deserialize`, ni constructeur public : seule
/// [`ConsentRegistry::authorize_capture`] la délivre, et [`AnalysisLock::begin_capture`] la
/// consomme.
#[derive(Debug, PartialEq, Eq)]
pub struct CaptureAuthorization {
    room: RoomId,
    terms_version: String,
    consent_given_at_ms: u64,
    terms_accepted_at_ms: u64,
    granted_at_ms: u64,
}

impl CaptureAuthorization {
    /// Salle autorisée.
    #[must_use]
    pub fn room(&self) -> &RoomId {
        &self.room
    }

    /// Version des conditions acceptée.
    #[must_use]
    pub fn terms_version(&self) -> &str {
        &self.terms_version
    }

    /// Date du consentement sur lequel repose l'autorisation.
    #[must_use]
    pub const fn consent_given_at_ms(&self) -> u64 {
        self.consent_given_at_ms
    }

    /// Date de l'acceptation des conditions sur laquelle repose l'autorisation.
    #[must_use]
    pub const fn terms_accepted_at_ms(&self) -> u64 {
        self.terms_accepted_at_ms
    }

    /// Date de délivrance.
    #[must_use]
    pub const fn granted_at_ms(&self) -> u64 {
        self.granted_at_ms
    }
}

/// État partagé du verrou.
#[derive(Debug, Default)]
struct LockState {
    /// Captures en cours : numéro d'inscription → identifiant de session.
    captures: BTreeMap<u64, String>,
    next_ticket: u64,
    /// Permis d'analyse ou d'entraînement vivants.
    permits: usize,
}

/// État observable du verrou.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LockStatus {
    /// Ni capture ni analyse.
    Idle,
    /// Capture(s) en cours : toute analyse est refusée.
    Capturing {
        /// Sessions en cours de capture.
        session_ids: Vec<String>,
    },
    /// Analyse(s) ou entraînement(s) en cours : toute capture est refusée.
    Analyzing {
        /// Nombre de permis vivants.
        permits: usize,
    },
}

/// Verrou d'analyse partagé entre la capture et l'analyse (décision D6). Les clones partagent
/// le même état.
#[derive(Debug, Clone, Default)]
pub struct AnalysisLock {
    state: Arc<Mutex<LockState>>,
}

impl AnalysisLock {
    /// Verrou indépendant (tests, outils). L'application utilise [`AnalysisLock::process_wide`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Verrou unique du processus : tous les appels partagent le même état.
    #[must_use]
    pub fn process_wide() -> Self {
        static LOCK: OnceLock<AnalysisLock> = OnceLock::new();
        LOCK.get_or_init(Self::new).clone()
    }

    fn state(&self) -> MutexGuard<'_, LockState> {
        // L'état reste cohérent même si un fil a paniqué en le tenant : chaque modification
        // est atomique sous le verrou.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// État courant.
    #[must_use]
    pub fn status(&self) -> LockStatus {
        let state = self.state();
        if !state.captures.is_empty() {
            LockStatus::Capturing {
                session_ids: state.captures.values().cloned().collect(),
            }
        } else if state.permits > 0 {
            LockStatus::Analyzing {
                permits: state.permits,
            }
        } else {
            LockStatus::Idle
        }
    }

    /// Démarre la capture de `session` (capture en direct, ouverte, commencée dans les
    /// 10 minutes suivant l'autorisation). Le verrou est tenu jusqu'à
    /// [`CaptureGuard::finish`] ou jusqu'à la destruction du garde.
    ///
    /// # Errors
    /// Voir [`CaptureError`] ; notamment [`CaptureError::AnalysisInProgress`].
    pub fn begin_capture(
        &self,
        authorization: CaptureAuthorization,
        session: Session,
    ) -> Result<CaptureGuard, CaptureError> {
        let session_id = || session.id.clone();
        if session.source != SessionSource::LiveCapture {
            return Err(CaptureError::NotLiveCapture {
                session_id: session_id(),
            });
        }
        if !session.is_open() {
            return Err(CaptureError::SessionAlreadyClosed {
                session_id: session_id(),
            });
        }
        if session.started_at_ms < authorization.granted_at_ms {
            return Err(CaptureError::StartedBeforeAuthorization {
                session_id: session_id(),
            });
        }
        if session.started_at_ms - authorization.granted_at_ms > AUTHORIZATION_VALIDITY_MS {
            return Err(CaptureError::AuthorizationExpired);
        }

        let mut state = self.state();
        if state.permits > 0 {
            return Err(CaptureError::AnalysisInProgress {
                permits: state.permits,
            });
        }
        if state.captures.values().any(|id| *id == session.id) {
            return Err(CaptureError::AlreadyCapturing {
                session_id: session_id(),
            });
        }
        let ticket = state.next_ticket;
        state.next_ticket += 1;
        state.captures.insert(ticket, session.id.clone());
        drop(state);

        Ok(CaptureGuard {
            registration: Registration {
                state: Arc::clone(&self.state),
                kind: RegistrationKind::Capture(ticket),
            },
            session,
            authorization,
        })
    }

    /// Délivre un permis d'analyser `session`.
    ///
    /// # Errors
    /// [`AnalysisDenied::CaptureInProgress`] si une capture est en cours (quelle que soit la
    /// session) ; [`AnalysisDenied::SessionOpen`] si `session` n'est pas close.
    pub fn authorize_analysis<'s>(
        &self,
        session: &'s Session,
    ) -> Result<AnalysisPermit<'s>, AnalysisDenied> {
        let registration = self.register_permit()?;
        if session.is_open() {
            // `registration` est libéré ici : le permis n'est pas délivré.
            return Err(AnalysisDenied::SessionOpen {
                session_id: session.id.clone(),
            });
        }
        Ok(AnalysisPermit {
            session,
            _registration: registration,
        })
    }

    /// Délivre un permis d'entraînement (exercices, quiz, simulations).
    ///
    /// # Errors
    /// [`AnalysisDenied::CaptureInProgress`] si une capture est en cours.
    pub fn authorize_training(&self) -> Result<TrainingPermit, AnalysisDenied> {
        Ok(TrainingPermit {
            _registration: self.register_permit()?,
        })
    }

    fn register_permit(&self) -> Result<Registration, AnalysisDenied> {
        let mut state = self.state();
        if !state.captures.is_empty() {
            return Err(AnalysisDenied::CaptureInProgress {
                session_ids: state.captures.values().cloned().collect(),
            });
        }
        state.permits += 1;
        Ok(Registration {
            state: Arc::clone(&self.state),
            kind: RegistrationKind::Permit,
        })
    }
}

/// Inscription dans le verrou, libérée à la destruction.
#[derive(Debug)]
struct Registration {
    state: Arc<Mutex<LockState>>,
    kind: RegistrationKind,
}

#[derive(Debug, Clone, Copy)]
enum RegistrationKind {
    Capture(u64),
    Permit,
}

impl Drop for Registration {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        match self.kind {
            RegistrationKind::Capture(ticket) => {
                state.captures.remove(&ticket);
            }
            RegistrationKind::Permit => state.permits = state.permits.saturating_sub(1),
        }
    }
}

/// Capture en cours. Tant qu'il existe, toute analyse est refusée.
///
/// [`CaptureGuard::finish`] clôt la session et libère le verrou. Un garde détruit sans
/// `finish` (plantage, abandon) libère aussi le verrou, mais sa session reste ouverte et donc
/// non analysable.
#[derive(Debug)]
pub struct CaptureGuard {
    registration: Registration,
    session: Session,
    authorization: CaptureAuthorization,
}

impl CaptureGuard {
    /// Session en cours de capture.
    #[must_use]
    pub const fn session(&self) -> &Session {
        &self.session
    }

    /// Autorisation sur laquelle repose la capture.
    #[must_use]
    pub const fn authorization(&self) -> &CaptureAuthorization {
        &self.authorization
    }

    /// Ajoute une main retranscrite à la session.
    pub fn record_hand(&mut self, hand_id: impl Into<String>) {
        self.session.hand_ids.push(hand_id.into());
    }

    /// Clôt la session à `ended_at_ms` (ramené au début si l'horloge a reculé), libère le
    /// verrou et rend la session, désormais analysable.
    #[must_use]
    pub fn finish(self, ended_at_ms: u64) -> Session {
        let Self {
            registration,
            mut session,
            ..
        } = self;
        session.ended_at_ms = Some(ended_at_ms.max(session.started_at_ms));
        drop(registration);
        session
    }
}

/// Permis d'analyse stratégique d'une session close. Les modules d'analyse l'exigent en
/// paramètre ; tant qu'il existe, aucune capture ne peut démarrer.
#[derive(Debug)]
pub struct AnalysisPermit<'s> {
    session: &'s Session,
    _registration: Registration,
}

impl<'s> AnalysisPermit<'s> {
    /// Session autorisée à l'analyse (close).
    #[must_use]
    pub const fn session(&self) -> &'s Session {
        self.session
    }
}

/// Permis d'entraînement (exercices, quiz, simulations). Tant qu'il existe, aucune capture
/// ne peut démarrer.
#[derive(Debug)]
pub struct TrainingPermit {
    _registration: Registration,
}

#[cfg(test)]
mod tests {
    use super::*;
    use stratyx_poker::GameFormat;

    const NOW: u64 = 1_800_000_000_000;

    fn winamax_terms(version: &str) -> RoomTerms {
        RoomTerms {
            room: RoomId::new("Winamax").unwrap(),
            version: version.into(),
        }
    }

    fn session(id: &str, source: SessionSource, started: u64, ended: Option<u64>) -> Session {
        Session {
            id: id.into(),
            source,
            format: GameFormat::EXPRESSO,
            started_at_ms: started,
            ended_at_ms: ended,
            hand_ids: vec![],
            tournament: None,
        }
    }

    fn ready_registry() -> ConsentRegistry {
        let mut registry = ConsentRegistry::new();
        registry.record_consent("consentement-v1", NOW - 1000);
        registry.accept_terms(&winamax_terms("2026-09"), NOW - 500);
        registry
    }

    #[test]
    fn room_ids_are_normalized_and_validated() {
        assert_eq!(
            RoomId::new("  PokerStars.FR ").unwrap().as_str(),
            "pokerstars.fr"
        );
        for bad in ["", "   ", "salle avec espaces", "../etc", &"x".repeat(65)] {
            assert!(RoomId::new(bad).is_err(), "{bad:?}");
        }
        assert!(serde_json::from_str::<RoomId>("\"a b\"").is_err());
    }

    #[test]
    fn capture_requires_consent_and_current_terms() {
        let terms = winamax_terms("2026-09");
        let mut registry = ConsentRegistry::new();
        assert_eq!(
            registry.authorize_capture(&terms, NOW).unwrap_err(),
            CaptureError::ConsentMissing
        );

        registry.record_consent("consentement-v1", NOW);
        assert!(matches!(
            registry.authorize_capture(&terms, NOW),
            Err(CaptureError::TermsNotAccepted { .. })
        ));

        registry.accept_terms(&winamax_terms("2026-01"), NOW);
        let err = registry.authorize_capture(&terms, NOW).unwrap_err();
        assert!(matches!(err, CaptureError::TermsOutdated { .. }));
        assert!(err.to_string().contains("2026-01"));

        registry.accept_terms(&terms, NOW + 1);
        let auth = registry.authorize_capture(&terms, NOW + 2).unwrap();
        assert_eq!(auth.room().as_str(), "winamax");
        assert_eq!(auth.terms_version(), "2026-09");
        assert_eq!(auth.terms_accepted_at_ms(), NOW + 1);

        // Une autre salle exige sa propre acceptation.
        let other = RoomTerms {
            room: RoomId::new("pokerstars").unwrap(),
            version: "1".into(),
        };
        assert!(matches!(
            registry.authorize_capture(&other, NOW),
            Err(CaptureError::TermsNotAccepted { .. })
        ));

        registry.withdraw_consent(NOW + 3).unwrap();
        assert_eq!(
            registry.authorize_capture(&terms, NOW + 4).unwrap_err(),
            CaptureError::ConsentWithdrawn
        );
        assert_eq!(
            registry.withdraw_consent(NOW + 5),
            Err(CaptureError::ConsentMissing)
        );
        assert_eq!(
            registry.consents().len(),
            1,
            "le retrait s'ajoute à l'historique"
        );

        let json = serde_json::to_string(&registry).unwrap();
        assert_eq!(
            serde_json::from_str::<ConsentRegistry>(&json).unwrap(),
            registry
        );
    }

    #[test]
    fn analysis_is_refused_while_capturing_d6() {
        let lock = AnalysisLock::new();
        let registry = ready_registry();
        let auth = registry
            .authorize_capture(&winamax_terms("2026-09"), NOW)
            .unwrap();
        let past = session(
            "hier",
            SessionSource::LiveCapture,
            NOW - 90_000,
            Some(NOW - 10_000),
        );

        let mut guard = lock
            .begin_capture(
                auth,
                session("live", SessionSource::LiveCapture, NOW + 1, None),
            )
            .unwrap();
        guard.record_hand("main-1");
        assert_eq!(
            lock.status(),
            LockStatus::Capturing {
                session_ids: vec!["live".into()]
            }
        );

        // Même une session close et ancienne ne peut pas être analysée pendant la capture.
        let denied = lock.authorize_analysis(&past).unwrap_err();
        assert_eq!(
            denied,
            AnalysisDenied::CaptureInProgress {
                session_ids: vec!["live".into()]
            }
        );
        assert!(denied.to_string().contains("D6"));
        assert!(lock.authorize_training().is_err());
        // Un clone du verrou partage l'état.
        assert!(lock.clone().authorize_analysis(&past).is_err());

        let closed = guard.finish(NOW + 60_000);
        assert!(!closed.is_open());
        assert_eq!(closed.hand_ids, ["main-1"]);
        assert_eq!(lock.status(), LockStatus::Idle);

        let permit = lock.authorize_analysis(&closed).unwrap();
        assert_eq!(permit.session().id, "live");
        assert_eq!(lock.status(), LockStatus::Analyzing { permits: 1 });
        drop(permit);
        assert_eq!(lock.status(), LockStatus::Idle);
    }

    #[test]
    fn open_session_is_never_analyzable() {
        let lock = AnalysisLock::new();
        let open = session("orpheline", SessionSource::VideoImport, NOW, None);
        assert_eq!(
            lock.authorize_analysis(&open).unwrap_err(),
            AnalysisDenied::SessionOpen {
                session_id: "orpheline".into()
            }
        );
        assert_eq!(lock.status(), LockStatus::Idle, "aucun permis fantôme");
    }

    #[test]
    fn capture_waits_for_analysis_to_end() {
        let lock = AnalysisLock::new();
        let registry = ready_registry();
        let terms = winamax_terms("2026-09");
        let training = lock.authorize_training().unwrap();
        let err = lock
            .begin_capture(
                registry.authorize_capture(&terms, NOW).unwrap(),
                session("live", SessionSource::LiveCapture, NOW, None),
            )
            .unwrap_err();
        assert_eq!(err, CaptureError::AnalysisInProgress { permits: 1 });
        drop(training);
        assert!(
            lock.begin_capture(
                registry.authorize_capture(&terms, NOW).unwrap(),
                session("live", SessionSource::LiveCapture, NOW, None),
            )
            .is_ok()
        );
    }

    #[test]
    fn begin_capture_validations() {
        let lock = AnalysisLock::new();
        let registry = ready_registry();
        let terms = winamax_terms("2026-09");
        let auth = || registry.authorize_capture(&terms, NOW).unwrap();

        let cases = [
            (
                session("import", SessionSource::VideoImport, NOW, None),
                CaptureError::NotLiveCapture {
                    session_id: "import".into(),
                },
            ),
            (
                session("close", SessionSource::LiveCapture, NOW, Some(NOW + 1)),
                CaptureError::SessionAlreadyClosed {
                    session_id: "close".into(),
                },
            ),
            (
                session("tot", SessionSource::LiveCapture, NOW - 1, None),
                CaptureError::StartedBeforeAuthorization {
                    session_id: "tot".into(),
                },
            ),
            (
                session(
                    "tard",
                    SessionSource::LiveCapture,
                    NOW + AUTHORIZATION_VALIDITY_MS + 1,
                    None,
                ),
                CaptureError::AuthorizationExpired,
            ),
        ];
        for (s, expected) in cases {
            assert_eq!(lock.begin_capture(auth(), s).unwrap_err(), expected);
        }

        let _first = lock
            .begin_capture(
                auth(),
                session("live", SessionSource::LiveCapture, NOW, None),
            )
            .unwrap();
        assert!(matches!(
            lock.begin_capture(
                auth(),
                session("live", SessionSource::LiveCapture, NOW, None)
            ),
            Err(CaptureError::AlreadyCapturing { .. })
        ));
        // Plusieurs tables en parallèle : autorisé, toutes bloquent l'analyse.
        let second = lock
            .begin_capture(
                auth(),
                session("table-2", SessionSource::LiveCapture, NOW, None),
            )
            .unwrap();
        drop(second);
        assert_eq!(
            lock.status(),
            LockStatus::Capturing {
                session_ids: vec!["live".into()]
            }
        );
    }

    #[test]
    fn dropped_guard_releases_lock_but_session_stays_unanalyzable() {
        let lock = AnalysisLock::new();
        let registry = ready_registry();
        let auth = registry
            .authorize_capture(&winamax_terms("2026-09"), NOW)
            .unwrap();
        let guard = lock
            .begin_capture(
                auth,
                session("plantage", SessionSource::LiveCapture, NOW, None),
            )
            .unwrap();
        let unfinished = guard.session().clone();
        drop(guard);
        assert_eq!(lock.status(), LockStatus::Idle);
        assert!(matches!(
            lock.authorize_analysis(&unfinished),
            Err(AnalysisDenied::SessionOpen { .. })
        ));
    }

    #[test]
    fn finish_clamps_backward_clock() {
        let lock = AnalysisLock::new();
        let registry = ready_registry();
        let auth = registry
            .authorize_capture(&winamax_terms("2026-09"), NOW)
            .unwrap();
        let guard = lock
            .begin_capture(
                auth,
                session("s", SessionSource::LiveCapture, NOW + 5, None),
            )
            .unwrap();
        assert_eq!(guard.finish(NOW).ended_at_ms, Some(NOW + 5));
    }

    #[test]
    fn lock_is_shared_across_threads() {
        let lock = AnalysisLock::new();
        let registry = ready_registry();
        let auth = registry
            .authorize_capture(&winamax_terms("2026-09"), NOW)
            .unwrap();
        let guard = lock
            .begin_capture(auth, session("live", SessionSource::LiveCapture, NOW, None))
            .unwrap();
        let past = session("hier", SessionSource::LiveCapture, 0, Some(1));
        let remote = lock.clone();
        let denied = std::thread::spawn(move || remote.authorize_analysis(&past).is_err())
            .join()
            .unwrap();
        assert!(denied);
        drop(guard);
        assert!(Arc::ptr_eq(
            &AnalysisLock::process_wide().state,
            &AnalysisLock::process_wide().state
        ));
    }
}
