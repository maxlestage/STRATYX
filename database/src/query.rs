//! Recherche de mains par filtres combinables, avec pagination.

use serde::{Deserialize, Serialize};
use stratyx_poker::game::BettingStructure;
use stratyx_poker::{GameFormat, GameType};

use crate::error::{DatabaseError, Result};
use crate::space::Space;
use crate::sql::{int_at, text_at, to_u64};
use crate::status::{HandSummary, ValidationKind};
use crate::store::{Database, from_json};

/// Résultat du héros sur une main résolue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeroResult {
    /// Gain net strictement positif.
    Won,
    /// Perte nette.
    Lost,
    /// Résultat nul (ex. : pot partagé à égalité de mises).
    BrokeEven,
}

/// Ordre des résultats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    /// Plus récentes d'abord ; les mains sans date en dernier.
    #[default]
    NewestFirst,
    /// Plus anciennes d'abord ; les mains sans date en dernier.
    OldestFirst,
}

/// Critères de recherche. Tous les filtres renseignés doivent être satisfaits.
///
/// ```
/// use stratyx_database::{HandQuery, HeroResult};
/// let q = HandQuery::new()
///     .player("Villain")
///     .hero_result(HeroResult::Lost)
///     .showdown(true)
///     .page(0, 20);
/// assert_eq!(q.limit, Some(20));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct HandQuery {
    /// Espace de mains ; toutes les bases de mains si absent.
    pub space: Option<Space>,
    /// Joueur servi dans la main (pseudonyme exact).
    pub player: Option<String>,
    /// Format exact.
    pub format: Option<GameFormat>,
    /// Type de partie (cash game, tournoi…), quel que soit le reste du format.
    pub game_type: Option<GameType>,
    pub session_id: Option<String>,
    pub site: Option<String>,
    /// Début de période (inclus), en ms depuis l'époque Unix. Exclut les mains sans date.
    pub played_from_ms: Option<u64>,
    /// Fin de période (exclue), en ms depuis l'époque Unix. Exclut les mains sans date.
    pub played_until_ms: Option<u64>,
    /// Résultat du héros ; exclut les mains non résolues ou sans héros.
    pub hero_result: Option<HeroResult>,
    /// `Some(true)` : mains à revoir (détection incertaine ou main non validée).
    pub needs_review: Option<bool>,
    pub validation: Option<ValidationKind>,
    /// Présence d'un abattage.
    pub showdown: Option<bool>,
    pub order: SortOrder,
    /// Nombre de résultats à sauter.
    pub offset: usize,
    /// Nombre maximal de résultats ; tous si absent.
    pub limit: Option<usize>,
}

/// Une page de résultats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandPage {
    /// Nombre total de mains correspondant aux filtres (toutes pages confondues).
    pub total: usize,
    pub items: Vec<HandSummary>,
}

impl HandQuery {
    /// Requête sans filtre : toutes les mains, sans limite.
    #[must_use]
    pub fn new() -> HandQuery {
        HandQuery::default()
    }

    /// Restreint la recherche à un espace de mains.
    #[must_use]
    pub fn space(mut self, space: Space) -> Self {
        self.space = Some(space);
        self
    }

    /// Mains où ce joueur est servi.
    #[must_use]
    pub fn player(mut self, name: impl Into<String>) -> Self {
        self.player = Some(name.into());
        self
    }

    /// Mains de ce format exact.
    #[must_use]
    pub fn format(mut self, format: GameFormat) -> Self {
        self.format = Some(format);
        self
    }

    /// Mains de ce type de partie.
    #[must_use]
    pub fn game_type(mut self, game_type: GameType) -> Self {
        self.game_type = Some(game_type);
        self
    }

    /// Mains de cette session.
    #[must_use]
    pub fn session(mut self, id: impl Into<String>) -> Self {
        self.session_id = Some(id.into());
        self
    }

    /// Mains de cette salle (`meta.site`).
    #[must_use]
    pub fn site(mut self, site: impl Into<String>) -> Self {
        self.site = Some(site.into());
        self
    }

    /// Période `[from, until[` en ms depuis l'époque Unix.
    #[must_use]
    pub fn between(mut self, from_ms: u64, until_ms: u64) -> Self {
        self.played_from_ms = Some(from_ms);
        self.played_until_ms = Some(until_ms);
        self
    }

    /// Mains que le héros a gagnées, perdues ou terminées à l'équilibre.
    #[must_use]
    pub fn hero_result(mut self, result: HeroResult) -> Self {
        self.hero_result = Some(result);
        self
    }

    /// Mains à revoir (`true`) ou validées sans réserve (`false`).
    #[must_use]
    pub fn needs_review(mut self, yes: bool) -> Self {
        self.needs_review = Some(yes);
        self
    }

    /// Mains de ce statut de validation.
    #[must_use]
    pub fn validation(mut self, kind: ValidationKind) -> Self {
        self.validation = Some(kind);
        self
    }

    /// Mains avec (`true`) ou sans (`false`) abattage.
    #[must_use]
    pub fn showdown(mut self, yes: bool) -> Self {
        self.showdown = Some(yes);
        self
    }

    /// Ordre chronologique des résultats.
    #[must_use]
    pub fn order(mut self, order: SortOrder) -> Self {
        self.order = order;
        self
    }

    /// Page de `limit` résultats à partir du rang `offset` (à partir de 0).
    #[must_use]
    pub fn page(mut self, offset: usize, limit: usize) -> Self {
        self.offset = offset;
        self.limit = Some(limit);
        self
    }

    /// Vrai si le résumé satisfait tous les filtres.
    #[must_use]
    pub fn matches(&self, s: &HandSummary) -> bool {
        let opt_eq = |filter: &Option<String>, value: &Option<String>| {
            filter.as_ref().is_none_or(|f| value.as_ref() == Some(f))
        };
        let in_period = || {
            if self.played_from_ms.is_none() && self.played_until_ms.is_none() {
                return true;
            }
            s.played_at_ms.is_some_and(|t| {
                self.played_from_ms.is_none_or(|from| t >= from)
                    && self.played_until_ms.is_none_or(|until| t < until)
            })
        };
        self.space.is_none_or(|x| x == s.space)
            && self.player.as_ref().is_none_or(|p| s.players.contains(p))
            && self.format.is_none_or(|f| f == s.format)
            && self.game_type.is_none_or(|g| g == s.format.game_type)
            && opt_eq(&self.session_id, &s.session_id)
            && opt_eq(&self.site, &s.site)
            && in_period()
            && self.hero_result.is_none_or(|r| {
                s.hero_net.is_some_and(|n| match r {
                    HeroResult::Won => n > 0,
                    HeroResult::Lost => n < 0,
                    HeroResult::BrokeEven => n == 0,
                })
            })
            && self
                .needs_review
                .is_none_or(|r| s.status.needs_review() == r)
            && self
                .validation
                .is_none_or(|v| s.status.validation.kind() == v)
            && self.showdown.is_none_or(|v| s.showdown == v)
    }
}

/// Début de la clé d'index d'un format : le type de partie.
fn game_key(g: GameType) -> &'static str {
    match g {
        GameType::CashGame => "cash_game",
        GameType::Tournament => "tournament",
        GameType::SitAndGo => "sit_and_go",
        GameType::Expresso => "expresso",
    }
}

/// Clé d'index stable d'un format : `cash_game-6-nl`, `tournament-9-fl4c`…
pub(crate) fn format_key(f: GameFormat) -> String {
    let game = game_key(f.game_type);
    let structure = match f.structure {
        BettingStructure::NoLimit => "nl".to_string(),
        BettingStructure::PotLimit => "pl".to_string(),
        BettingStructure::FixedLimit {
            max_bets,
            uncapped_heads_up,
        } => format!(
            "fl{}{}",
            max_bets.map_or_else(|| "x".to_string(), |m| m.to_string()),
            if uncapped_heads_up { "u" } else { "c" }
        ),
    };
    format!("{game}-{}-{structure}", f.table_size)
}

/// Ordre des résultats : par date (voir [`SortOrder`], sans date en dernier), puis par
/// identifiant.
fn by_date_then_id(
    order: SortOrder,
    (a_date, a_id): (Option<u64>, &str),
    (b_date, b_id): (Option<u64>, &str),
) -> std::cmp::Ordering {
    let date = match (a_date, b_date) {
        (Some(x), Some(y)) => match order {
            SortOrder::NewestFirst => y.cmp(&x),
            SortOrder::OldestFirst => x.cmp(&y),
        },
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    };
    date.then_with(|| a_id.cmp(b_id))
}

/// Filtres d'une requête traduits en conditions SQL sur les colonnes indexées de `main`.
struct Conditions {
    sql: String,
    params: Vec<serde_json::Value>,
    /// Tous les filtres sont traduits : le tri et la pagination peuvent se faire en SQL.
    complete: bool,
}

impl Conditions {
    fn of(query: &HandQuery) -> Conditions {
        let mut conditions: Vec<&str> = Vec::new();
        let mut params: Vec<serde_json::Value> = Vec::new();
        if let Some(space) = query.space {
            conditions.push("espace = ?");
            params.push(space.code().into());
        }
        if let Some(session) = &query.session_id {
            conditions.push("session = ?");
            params.push(session.as_str().into());
        }
        if let Some(player) = &query.player {
            conditions.push("id IN (SELECT main FROM main_joueur WHERE joueur = ?)");
            params.push(player.as_str().into());
        }
        if let Some(review) = query.needs_review {
            conditions.push("a_revoir = ?");
            params.push(u8::from(review).into());
        }
        if let Some(kind) = query.validation {
            conditions.push("validation = ?");
            params.push(kind.key().into());
        }
        if let Some(format) = query.format {
            conditions.push("format = ?");
            params.push(format_key(format).into());
        }
        if let Some(game) = query.game_type {
            // Les clés de type de partie ne contiennent ni `%` ni `-` : le préfixe suffit.
            conditions.push("format LIKE ?");
            params.push(format!("{}-%", game_key(game)).into());
        }
        if let Some(from) = query.played_from_ms {
            conditions.push("joue_le >= ?");
            params.push(from.into());
        }
        if let Some(until) = query.played_until_ms {
            conditions.push("joue_le < ?");
            params.push(until.into());
        }
        let sql = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };
        Conditions {
            sql,
            params,
            complete: query.site.is_none()
                && query.hero_result.is_none()
                && query.showdown.is_none(),
        }
    }
}

impl Database {
    /// Recherche des mains. Les résultats sont triés par date (voir [`SortOrder`]), puis par
    /// identifiant ; la pagination s'applique après le tri.
    ///
    /// Quand tous les filtres portent sur des colonnes indexées (espace, joueur, session,
    /// format, type de partie, statut, période), la base les applique, le tri se fait sur
    /// l'identifiant et la date seuls, et seuls les résumés de la page sont relus. Sinon
    /// (salle, résultat du héros, abattage), chaque résumé candidat est relu et filtré.
    pub fn search(&self, query: &HandQuery) -> Result<HandPage> {
        let c = Conditions::of(query);
        if c.complete {
            self.search_in_base(query, &c)
        } else {
            self.search_by_scan(query, &c)
        }
    }

    fn search_in_base(&self, query: &HandQuery, c: &Conditions) -> Result<HandPage> {
        let tx = self.read()?;
        // Un seul passage sur les candidats, sans relire leur résumé : identifiant et date.
        let mut keys = tx
            .rows(&format!("SELECT id, joue_le FROM main{}", c.sql), &c.params)?
            .iter()
            .map(|row| Ok((text_at(row, 0)?, int_at(row, 1)?.map(to_u64).transpose()?)))
            .collect::<Result<Vec<(String, Option<u64>)>>>()?;
        keys.sort_by(|a, b| by_date_then_id(query.order, (a.1, &a.0), (b.1, &b.0)));
        let total = keys.len();
        let page: Vec<String> = keys
            .into_iter()
            .skip(query.offset)
            .take(query.limit.unwrap_or(usize::MAX))
            .map(|(id, _)| id)
            .collect();
        if page.is_empty() {
            return Ok(HandPage {
                total,
                items: Vec::new(),
            });
        }
        // Puis les résumés de la page seulement, remis dans l'ordre du tri.
        let marks = vec!["?"; page.len()].join(", ");
        let params: Vec<serde_json::Value> = page.iter().map(|id| id.as_str().into()).collect();
        let mut summaries: std::collections::HashMap<String, HandSummary> = tx
            .rows(
                &format!("SELECT id, resume FROM main WHERE id IN ({marks})"),
                &params,
            )?
            .iter()
            .map(|row| Ok((text_at(row, 0)?, from_json(text_at(row, 1)?.as_bytes())?)))
            .collect::<Result<_>>()?;
        let items = page
            .iter()
            .map(|id| {
                summaries
                    .remove(id)
                    .ok_or_else(|| DatabaseError::Corrupted(format!("résumé de « {id} » absent")))
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(HandPage { total, items })
    }

    fn search_by_scan(&self, query: &HandQuery, c: &Conditions) -> Result<HandPage> {
        let mut items = Vec::new();
        let sql = format!("SELECT resume FROM main{}", c.sql);
        for json in self.read()?.texts(&sql, &c.params)? {
            let s: HandSummary = from_json(json.as_bytes())?;
            if query.matches(&s) {
                items.push(s);
            }
        }
        items.sort_by(|a, b| {
            by_date_then_id(
                query.order,
                (a.played_at_ms, &a.id),
                (b.played_at_ms, &b.id),
            )
        });
        let total = items.len();
        let items = items
            .into_iter()
            .skip(query.offset)
            .take(query.limit.unwrap_or(usize::MAX))
            .collect();
        Ok(HandPage { total, items })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::space::Space;

    #[test]
    fn format_keys_are_distinct() {
        let a = format_key(GameFormat::EXPRESSO);
        let b = format_key(GameFormat::TOURNAMENT_6MAX);
        let c = format_key(
            GameFormat::new(GameType::CashGame, 9, BettingStructure::FIXED_LIMIT_CASH).unwrap(),
        );
        let d = format_key(
            GameFormat::new(
                GameType::CashGame,
                9,
                BettingStructure::FIXED_LIMIT_TOURNAMENT,
            )
            .unwrap(),
        );
        assert_eq!(a, "expresso-3-nl");
        assert_eq!(b, "tournament-6-nl");
        assert_eq!(c, "cash_game-9-fl4u");
        assert_ne!(c, d);
    }

    /// La recherche faite par la base donne exactement le résultat du parcours un à un, pour
    /// toutes les combinaisons de filtres indexés, d'ordre et de page.
    #[test]
    fn search_in_base_matches_the_scan() {
        use crate::text::{ImportOptions, import_pokerstars};

        let db = Database::in_memory().unwrap();
        let mut hands = Vec::new();
        for text in [
            include_str!("../tests/data/pokerstars_cash_nl6.txt"),
            include_str!("../tests/data/pokerstars_tournament_antes.txt"),
            include_str!("../tests/data/pokerstars_fixed_limit.txt"),
        ] {
            for h in import_pokerstars(text, &ImportOptions::default()) {
                hands.push(h.unwrap().record);
            }
        }
        // Des dates en double, des mains sans date, des sessions : le tri doit départager.
        for (i, h) in hands.iter_mut().enumerate() {
            h.meta.played_at_ms = (i % 4 != 0).then_some((i as u64 % 3) * 1_000);
            h.meta.session_id = Some(format!("s{}", i % 2));
        }
        for h in &hands {
            db.put_hand(h).unwrap();
        }
        let player = hands[0].seats[0].player.clone();
        let mut queries = Vec::new();
        for order in [SortOrder::NewestFirst, SortOrder::OldestFirst] {
            for (offset, limit) in [(0, None), (0, Some(2)), (1, Some(3)), (50, Some(5))] {
                let base = HandQuery {
                    order,
                    offset,
                    limit,
                    ..HandQuery::new()
                };
                queries.push(base.clone());
                queries.push(base.clone().player(player.clone()));
                queries.push(base.clone().session("s1"));
                queries.push(base.clone().space(Space::Holdem));
                queries.push(base.clone().game_type(GameType::Tournament));
                queries.push(base.clone().format(hands[0].format));
                queries.push(base.clone().between(500, 2_000));
                queries.push(base.clone().needs_review(false));
                queries.push(base.clone().validation(ValidationKind::Valid));
            }
        }
        for q in &queries {
            let c = Conditions::of(q);
            assert!(c.complete);
            let fast = db.search_in_base(q, &c).unwrap();
            let scan = db.search_by_scan(q, &c).unwrap();
            assert_eq!(fast, scan, "{q:?}");
        }
        assert!(db.search(&HandQuery::new()).unwrap().total == hands.len());
    }
}
