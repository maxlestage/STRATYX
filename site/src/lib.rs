//! Ce que le serveur et le navigateur partagent : les îlots du site.
//!
//! Le serveur les rend en HTML (src/main.rs) ; compilé en WebAssembly, ce fichier les rend
//! interactifs. Les calculs sont ceux du vrai moteur STRATYX (`stratyx-poker`), exécuté
//! dans le navigateur : évaluation des mains, équité, ranges, ICM.

use paquets::prelude::*;
use stratyx_poker::equity::{EquityOptions, equity};
use stratyx_poker::eval::best_five;
use stratyx_poker::icm::icm_equities;
use stratyx_poker::{Card, CardSet, HandClass, HoleCards, Range, Rank};
use stratyx_shared::Chips;

#[cfg(target_arch = "wasm32")]
mod mouvement;
#[cfg(target_arch = "wasm32")]
mod scenes;

/// Les îlots, sous le nom donné à `island(...)`.
pub const ISLANDS: [(&str, paquets::active::IslandFn); 7] = [
    ("mouvement", mouvement),
    ("hero3d", hero3d),
    ("table3d", table3d),
    ("equite", equite),
    ("grille", grille),
    ("icm", icm),
    ("jetons", jetons),
];

/// Les formats de table de la démo : nom, nombre de joueurs.
pub const FORMATS: [(&str, usize); 3] = [
    ("Expresso 3-Max", 3),
    ("Tournois 6-Max", 6),
    ("Table pleine", 9),
];

/// La main de départ de la démo, identique sur le serveur et dans le navigateur :
/// A♠ K♠, puis Q♠ J♠ 7♦ 2♣ T♠.
pub const MAIN_INITIALE: [u8; 7] = [51, 47, 43, 39, 21, 0, 35];

// ————————————————————————————————————————————————————————————————————————————
// Outils communs
// ————————————————————————————————————————————————————————————————————————————

/// Un nombre au hasard dans [0, 1[ : celui du navigateur ; fixe sur le serveur.
fn hasard() -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Math::random()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        0.42
    }
}

/// `n` cartes différentes tirées du paquet (identifiants 0 à 51).
pub fn donner(n: usize) -> Vec<u8> {
    let mut paquet: Vec<u8> = (0..52).collect();
    for i in 0..n.min(52) {
        let j = i + (hasard() * (52 - i) as f64) as usize % (52 - i);
        paquet.swap(i, j);
    }
    paquet.truncate(n);
    paquet
}

/// Lance `f` un peu plus tard, pour que la page affiche d'abord « calcul en cours ».
fn plus_tard(f: impl FnOnce() + 'static) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        let rappel = wasm_bindgen::closure::Closure::once_into_js(f);
        if let Some(w) = web_sys::window() {
            let _ =
                w.set_timeout_with_callback_and_timeout_and_arguments_0(rappel.unchecked_ref(), 30);
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    f();
}

fn carte_de(id: u8) -> Option<Card> {
    Card::from_id(id)
}

/// Le rang affiché sur une carte (`T` devient `10`).
fn rang(r: Rank) -> String {
    match r.to_char() {
        'T' => "10".into(),
        c => c.to_string(),
    }
}

/// Le symbole et la couleur (rouge ou non) d'une couleur de carte.
fn symbole(c: Card) -> (&'static str, bool) {
    match c.to_string().chars().nth(1) {
        Some('h') => ("♥", true),
        Some('d') => ("♦", true),
        Some('c') => ("♣", false),
        _ => ("♠", false),
    }
}

/// Une carte à jouer en HTML (un dos quand `carte` est `None`).
pub fn carte_html(carte: Option<Card>) -> Element {
    match carte {
        Some(c) => {
            let (s, rouge) = symbole(c);
            div()
                .class(if rouge { "pcard red" } else { "pcard" })
                .attr("aria-label", c.to_string())
                .child(span().class("pc-r").text(rang(c.rank())))
                .child(span().class("pc-s").text(s))
                .child(span().class("pc-big").attr("aria-hidden", "true").text(s))
        }
        None => div().class("pcard back").attr("aria-hidden", "true"),
    }
}

fn pourcent(x: f64) -> String {
    format!("{:.1} %", x * 100.0).replace('.', ",")
}

/// L'équité d'une main contre une main au hasard, sur un board.
fn equite_contre_hasard(main: [Card; 2], board: CardSet) -> Option<f64> {
    let hole = HoleCards::new(main[0], main[1])?;
    let options = EquityOptions {
        samples: 30_000,
        ..EquityOptions::default()
    };
    equity(
        &[Range::single(hole), Range::full()],
        board,
        CardSet::EMPTY,
        &options,
    )
    .ok()
    .map(|r| r.players[0].equity)
}

/// L'analyse d'une main de la démo : ce que l'on a, et l'équité contre une main au hasard.
pub fn analyser(cartes: &[u8], etape: u8) -> (String, String) {
    let ids: Vec<Card> = cartes.iter().filter_map(|i| carte_de(*i)).collect();
    if ids.len() < 7 {
        return ("—".into(), "—".into());
    }
    let main = [ids[0], ids[1]];
    let visibles = [0usize, 3, 4, 5][etape.min(3) as usize];
    let board: CardSet = ids[2..2 + visibles]
        .iter()
        .fold(CardSet::EMPTY, |s, c| s | CardSet::from_card(*c));
    let titre = if visibles == 0 {
        match HoleCards::new(main[0], main[1]) {
            Some(h) => format!("Préflop · {}", h.class()),
            None => "Préflop".into(),
        }
    } else {
        let tout = board | CardSet::from_card(main[0]) | CardSet::from_card(main[1]);
        match best_five(tout) {
            Some((cinq, force)) => format!(
                "{} · {}",
                force.category(),
                cinq.iter()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            None => "—".into(),
        }
    };
    let eq = equite_contre_hasard(main, board)
        .map(|e| format!("{} d'équité contre une main au hasard", pourcent(e)))
        .unwrap_or_else(|| "—".into());
    (titre, eq)
}

// ————————————————————————————————————————————————————————————————————————————
// Liens entre les boutons d'un îlot et sa scène 3D
// ————————————————————————————————————————————————————————————————————————————

/// Quelle scène lancer dans un canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scene3d {
    /// Le fond du héros.
    Hero,
    /// La table de la démo.
    Table,
    /// La pluie de jetons.
    Jetons,
}

/// Ce que les boutons d'un îlot disent à sa scène.
#[derive(Clone, Copy)]
pub struct Liens {
    /// Index dans [`FORMATS`].
    pub format: State<usize>,
    /// 0 préflop, 1 flop, 2 turn, 3 river.
    pub etape: State<u8>,
    /// Les sept cartes de la main en cours.
    pub cartes: State<Vec<u8>>,
    /// Nombre de mains distribuées.
    pub donne: State<u32>,
    /// Jetons demandés depuis le début.
    pub lacher: State<u32>,
    /// Secousses demandées depuis le début.
    pub secousse: State<u32>,
}

impl Default for Liens {
    fn default() -> Self {
        Liens {
            format: use_state(1),
            etape: use_state(0),
            cartes: use_state(MAIN_INITIALE.to_vec()),
            donne: use_state(0),
            lacher: use_state(0),
            secousse: use_state(0),
        }
    }
}

/// Lance la scène dans le navigateur et rend ce qui l'arrête.
#[cfg(target_arch = "wasm32")]
fn lancer(
    canvas: &paquets::active::Dom,
    scene: Scene3d,
    info: State<u32>,
    liens: Liens,
) -> impl FnOnce() + use<> {
    let running = scenes::start(canvas, scene, info, liens);
    move || drop(running)
}

/// Rien à lancer sur le serveur.
#[cfg(not(target_arch = "wasm32"))]
fn lancer(
    _canvas: &paquets::active::Dom,
    _scene: Scene3d,
    _info: State<u32>,
    _liens: Liens,
) -> impl FnOnce() + use<> {
    || {}
}

// ————————————————————————————————————————————————————————————————————————————
// Les îlots
// ————————————————————————————————————————————————————————————————————————————

/// Les mouvements de la page (src/mouvement.rs) : un îlot invisible, présent sur chaque
/// page, qui charge le bundle WebAssembly et lance les animations en Rust.
pub fn mouvement(_props: &str) -> Node {
    span()
        .attr("hidden", "")
        .on_mount(|_| {
            #[cfg(target_arch = "wasm32")]
            mouvement::demarrer();
        })
        .into()
}

/// Le fond du héros : jetons et cartes en orbite, qui suivent le pointeur et le défilement.
pub fn hero3d(_props: &str) -> Node {
    let ips = use_state(0u32);
    let l = Liens::default();
    fragment([
        canvas()
            .class("hero-canvas")
            .attr("aria-hidden", "true")
            .use_action(move |canvas| lancer(canvas, Scene3d::Hero, ips, l)),
        p().class("hud")
            .attr("aria-hidden", "true")
            .child(span().class("hud-dot"))
            .text("paquets::three · WebGL 2 · ")
            .text_dyn(move || match ips.get() {
                0 => "—".into(),
                n => n.to_string(),
            })
            .text(" ips"),
    ])
}

/// La démo : une table en 3D, une main distribuée et analysée par le moteur STRATYX.
pub fn table3d(_props: &str) -> Node {
    let l = Liens::default();
    let ips = use_state(0u32);
    let (titre0, eq0) = analyser(&MAIN_INITIALE, 0);
    let titre = use_state(titre0);
    let eq = use_state(eq0);
    let calcul = use_state(false);
    let analyser_plus_tard = move || {
        calcul.set(true);
        plus_tard(move || {
            let (t, e) = analyser(&l.cartes.get(), l.etape.get());
            titre.set(t);
            eq.set(e);
            calcul.set(false);
        });
    };
    let visibles = move || [0usize, 3, 4, 5][l.etape.get().min(3) as usize];
    div()
        .class("demo")
        .child(
            div()
                .class("demo-stage")
                .child(
                    canvas()
                        .class("table-canvas")
                        .attr("role", "img")
                        .attr("aria-label", "Table de poker en 3D")
                        .use_action(move |canvas| lancer(canvas, Scene3d::Table, ips, l)),
                )
                .child(
                    p().class("stage-hint")
                        .text("↻ Glissez pour tourner autour de la table · ")
                        .text_dyn(move || match ips.get() {
                            0 => "—".into(),
                            n => format!("{n} ips"),
                        }),
                ),
        )
        .child(
            div()
                .class("demo-panel")
                .child(p().class("muted label").text("Format"))
                .child(
                    div()
                        .class("chips")
                        .attr("role", "radiogroup")
                        .attr("aria-label", "Format de table")
                        .children(FORMATS.iter().enumerate().map(|(i, (nom, _))| {
                            button()
                                .class("chip")
                                .attr("type", "button")
                                .attr("role", "radio")
                                .attr_dyn("aria-checked", move || (l.format.get() == i).to_string())
                                .on_click(move |_| l.format.set(i))
                                .text(*nom)
                        })),
                )
                .child(p().class("muted label").text("Votre main"))
                .child(div().class("hand").children_dyn(move || {
                    let cartes = l.cartes.get();
                    cartes
                        .iter()
                        .take(2)
                        .map(|id| Node::from(carte_html(carte_de(*id))))
                        .collect()
                }))
                .child(p().class("muted label").text("Board"))
                .child(div().class("board").children_dyn(move || {
                    let cartes = l.cartes.get();
                    let n = visibles();
                    (0..5)
                        .map(|i| {
                            let c = if i < n { cartes.get(2 + i).and_then(|id| carte_de(*id)) } else { None };
                            Node::from(carte_html(c).class_if("deal", move || i < n))
                        })
                        .collect()
                }))
                .child(
                    div()
                        .class("analysis")
                        .attr("aria-live", "polite")
                        .child(p().class("analysis-title").text_dyn(move || titre.get()))
                        .child(p().class("analysis-eq").text_dyn(move || {
                            if calcul.get() { "Calcul en cours…".into() } else { eq.get() }
                        })),
                )
                .child(
                    div()
                        .class("demo-actions")
                        .child(
                            button()
                                .class("btn btn-primary magnetic")
                                .attr("type", "button")
                                .on_click(move |_| {
                                    if l.etape.get() >= 3 {
                                        l.cartes.set(donner(7));
                                        l.donne.update(|d| *d += 1);
                                        l.etape.set(0);
                                    } else {
                                        l.etape.update(|e| *e += 1);
                                    }
                                    analyser_plus_tard();
                                })
                                .text_dyn(move || {
                                    match l.etape.get() {
                                        0 => "Voir le flop",
                                        1 => "Voir le turn",
                                        2 => "Voir la river",
                                        _ => "Nouvelle main",
                                    }
                                    .into()
                                }),
                        )
                        .child(
                            button()
                                .class("btn btn-ghost")
                                .attr("type", "button")
                                .on_click(move |_| {
                                    l.cartes.set(donner(7));
                                    l.donne.update(|d| *d += 1);
                                    l.etape.set(0);
                                    analyser_plus_tard();
                                })
                                .text("Redistribuer"),
                        ),
                )
                .child(p().class("fine").text(
                    "Évaluation et équité calculées par stratyx-poker, compilé en WebAssembly, dans votre navigateur.",
                )),
        )
        .into()
}

/// Le calculateur d'équité : deux ou trois ranges, un board, le moteur fait le reste.
pub fn equite(_props: &str) -> Node {
    let j1 = use_state(String::from("AhKh"));
    let j2 = use_state(String::from("QQ+,AKo"));
    let j3 = use_state(String::new());
    let board = use_state(String::new());
    let resultat: State<Vec<LigneEquite>> = use_state(Vec::new());
    let message = use_state(String::from("Choisissez les mains, puis lancez le calcul."));
    let calcul = use_state(false);
    let lancer_calcul = move || {
        calcul.set(true);
        message.set("Calcul en cours…".into());
        plus_tard(move || {
            match calculer_equite(&[j1.get(), j2.get(), j3.get()], &board.get()) {
                Ok((lignes, info)) => {
                    resultat.set(lignes);
                    message.set(info);
                }
                Err(e) => {
                    resultat.set(Vec::new());
                    message.set(format!("⚠️ {e}"));
                }
            }
            calcul.set(false);
        });
    };
    let champ = |id: &'static str, libelle: &'static str, aide: &'static str, s: State<String>| {
        div()
            .class("field")
            .child(label().attr("for", id).text(libelle))
            .child(
                input()
                    .id(id)
                    .attr("type", "text")
                    .attr("spellcheck", "false")
                    .attr("autocomplete", "off")
                    .attr("placeholder", aide)
                    .bind_value(s),
            )
    };
    div()
        .class("tool card")
        .child(
            form()
                .class("tool-form")
                .on_submit(move |e| {
                    e.prevent_default();
                    lancer_calcul();
                })
                .child(
                    div()
                        .class("row-2")
                        .child(champ("eq-j1", "Joueur 1", "AhKh, JJ+, AKs…", j1))
                        .child(champ("eq-j2", "Joueur 2", "QQ+,AKo", j2)),
                )
                .child(
                    div()
                        .class("row-2")
                        .child(champ("eq-j3", "Joueur 3 (facultatif)", "22+,A2s+", j3))
                        .child(champ(
                            "eq-board",
                            "Board (0, 3, 4 ou 5 cartes)",
                            "Qs7h2h",
                            board,
                        )),
                )
                .child(
                    div()
                        .class("presets")
                        .child(span().class("muted").text("Exemples :"))
                        .children(
                            [
                                ("Coinflip", "AhKh", "QsQd", ""),
                                ("Tirage couleur", "AhKh", "QQ+,AKo", "Qs7h2h"),
                                ("Multiway", "JJ+", "AKs,AKo", "22+"),
                            ]
                            .map(|(nom, a, b, bo)| {
                                button()
                                    .class("chip")
                                    .attr("type", "button")
                                    .on_click(move |_| {
                                        j1.set(a.into());
                                        j2.set(b.into());
                                        j3.set(if nom == "Multiway" {
                                            "22+".into()
                                        } else {
                                            String::new()
                                        });
                                        board.set(if nom == "Multiway" {
                                            String::new()
                                        } else {
                                            bo.into()
                                        });
                                    })
                                    .text(nom)
                            }),
                        ),
                )
                .child(
                    button()
                        .class("btn btn-primary magnetic")
                        .attr("type", "submit")
                        .bool_attr("disabled", move || calcul.get())
                        .text("Calculer l'équité"),
                ),
        )
        .child(div().class("bars").children_dyn(move || {
            resultat
                .get()
                .into_iter()
                .enumerate()
                .map(|(i, (nom, eq, win, tie))| {
                    Node::from(
                        div()
                            .class(format!("bar b{i}"))
                            .child(
                                p().class("bar-head")
                                    .child(strong().text(format!("Joueur {} · {nom}", i + 1)))
                                    .child(span().class("bar-val").text(pourcent(eq))),
                            )
                            .child(
                                div().class("bar-track").child(
                                    div()
                                        .class("bar-fill")
                                        .attr("style", format!("--v:{:.4}", eq)),
                                ),
                            )
                            .child(p().class("fine").text(format!(
                                "Victoire {} · Partage {}",
                                pourcent(win),
                                pourcent(tie)
                            ))),
                    )
                })
                .collect()
        }))
        .child(
            p().class("tool-info")
                .attr("aria-live", "polite")
                .text_dyn(move || message.get()),
        )
        .into()
}

/// Une ligne du calculateur d'équité : main ou range, équité, victoire, partage.
pub type LigneEquite = (String, f64, f64, f64);

/// Le calcul du calculateur d'équité.
pub fn calculer_equite(
    joueurs: &[String],
    board: &str,
) -> Result<(Vec<LigneEquite>, String), String> {
    let mut noms = Vec::new();
    let mut ranges = Vec::new();
    for j in joueurs {
        let j = j.trim();
        if j.is_empty() {
            continue;
        }
        ranges.push(Range::parse(j).map_err(|e| e.to_string())?);
        noms.push(j.to_owned());
    }
    let board = if board.trim().is_empty() {
        CardSet::EMPTY
    } else {
        CardSet::parse(board.trim()).map_err(|e| e.to_string())?
    };
    let options = EquityOptions {
        samples: 80_000,
        ..EquityOptions::default()
    };
    let r = equity(&ranges, board, CardSet::EMPTY, &options).map_err(|e| e.to_string())?;
    let methode = match r.method {
        stratyx_poker::equity::EquityMethod::Exact => "calcul exact",
        stratyx_poker::equity::EquityMethod::MonteCarlo => "Monte-Carlo",
    };
    let info = format!(
        "{} sur {} distributions · moteur stratyx-poker",
        methode,
        r.deals
            .to_string()
            .as_bytes()
            .rchunks(3)
            .rev()
            .map(|c| std::str::from_utf8(c).unwrap_or(""))
            .collect::<Vec<_>>()
            .join("\u{202f}")
    );
    Ok((
        noms.into_iter()
            .zip(r.players)
            .map(|(n, p)| (n, p.equity, p.win, p.tie))
            .collect(),
        info,
    ))
}

/// Les 169 classes de la grille, ligne par ligne (paires sur la diagonale, assorties
/// au-dessus, dépareillées en dessous).
pub fn classes_grille() -> Vec<HandClass> {
    let rangs: Vec<Rank> = Rank::ALL.into_iter().rev().collect();
    let mut out = Vec::with_capacity(169);
    for (i, hi) in rangs.iter().enumerate() {
        for (j, lo) in rangs.iter().enumerate() {
            out.push(match i.cmp(&j) {
                std::cmp::Ordering::Equal => HandClass::Pair(*hi),
                std::cmp::Ordering::Less => HandClass::Suited(*hi, *lo),
                std::cmp::Ordering::Greater => HandClass::Offsuit(*lo, *hi),
            });
        }
    }
    out
}

/// Le poids moyen de chaque classe de la grille dans une range, et sa part des 1326 mains.
pub fn poids_grille(texte: &str) -> Result<(Vec<f32>, f64, f64), String> {
    let range = Range::parse(texte).map_err(|e| e.to_string())?;
    let poids = classes_grille()
        .into_iter()
        .map(|c| {
            let combos = c.combos();
            combos.iter().map(|h| range.get(*h)).sum::<f32>() / combos.len().max(1) as f32
        })
        .collect();
    Ok((poids, range.combo_count(), range.fraction()))
}

/// La grille des ranges : 13 × 13 mains de départ, colorées en direct.
pub fn grille(_props: &str) -> Node {
    const DEPART: &str = "22+,A2s+,K9s+,Q9s+,J9s+,T9s,98s,87s,ATo+,KTo+,QTo+,JTo";
    let texte = use_state(String::from(DEPART));
    let (p0, c0, f0) = poids_grille(DEPART).unwrap_or((vec![0.0; 169], 0.0, 0.0));
    let poids = use_state(p0);
    let infos = use_state((c0, f0, String::new()));
    // La grille suit le texte : une range valide la redessine, une erreur s'affiche.
    effect(move || match poids_grille(&texte.get()) {
        Ok((p, c, f)) => {
            poids.set(p);
            infos.set((c, f, String::new()));
        }
        Err(e) => infos.update(|i| i.2 = e),
    });
    let classes = classes_grille();
    div()
        .class("tool card grille")
        .child(
            div()
                .class("field")
                .child(label().attr("for", "range").text("Range (syntaxe standard)"))
                .child(
                    input()
                        .id("range")
                        .attr("type", "text")
                        .attr("spellcheck", "false")
                        .attr("autocomplete", "off")
                        .bind_value(texte),
                ),
        )
        .child(
            div()
                .class("presets")
                .child(span().class("muted").text("Exemples :"))
                .children(
                    [
                        ("Premium", "QQ+,AKs,AKo"),
                        ("Ouverture UTG", "66+,ATs+,KTs+,QTs+,JTs,AJo+,KQo"),
                        ("Ouverture bouton", "22+,A2s+,K5s+,Q7s+,J7s+,T7s+,97s+,86s+,76s,65s,A5o+,K9o+,Q9o+,J9o+,T9o"),
                        ("Pondérée", "TT+,AK,99:0.5,AQs:0.5"),
                    ]
                    .map(|(nom, r)| {
                        button()
                            .class("chip")
                            .attr("type", "button")
                            .on_click(move |_| texte.set(r.into()))
                            .text(nom)
                    }),
                ),
        )
        .child(
            div()
                .class("matrix")
                .attr("role", "img")
                .attr("aria-label", "Grille des 169 mains de départ")
                .children(classes.into_iter().enumerate().map(|(i, c)| {
                    let genre = match c {
                        HandClass::Pair(_) => "cell pair",
                        HandClass::Suited(..) => "cell suited",
                        HandClass::Offsuit(..) => "cell offsuit",
                    };
                    div()
                        .class(genre)
                        .class_if("on", move || poids.with(|p| p[i] > 0.0))
                        .style_dyn("--w", move || format!("{:.3}", poids.with(|p| p[i])))
                        .text(c.to_string())
                })),
        )
        .child(p().class("tool-info").attr("aria-live", "polite").text_dyn(move || {
            let (c, f, e) = infos.get();
            if e.is_empty() {
                format!("{} combinaisons · {} des 1326 mains de départ", c.round(), pourcent(f))
            } else {
                format!("⚠️ {e}")
            }
        }))
        .into()
}

/// Le calcul ICM : tapis et structure de paiement → équité en argent de chaque joueur.
pub fn calculer_icm(tapis: &str, paiements: &str) -> Result<Vec<(u64, f64)>, String> {
    let tapis: Vec<u64> = tapis
        .split(',')
        .map(|t| t.trim().replace([' ', '\u{202f}'], ""))
        .filter(|t| !t.is_empty())
        .map(|t| {
            t.parse::<u64>()
                .map_err(|_| format!("tapis illisible : {t}"))
        })
        .collect::<Result<_, _>>()?;
    let paiements: Vec<f64> = paiements
        .split(',')
        .map(|t| t.trim().replace(',', "."))
        .filter(|t| !t.is_empty())
        .map(|t| {
            t.parse::<f64>()
                .map_err(|_| format!("paiement illisible : {t}"))
        })
        .collect::<Result<_, _>>()?;
    if tapis.len() > 9 {
        return Err("9 joueurs au plus dans cette démo".into());
    }
    let chips: Vec<Chips> = tapis.iter().map(|t| Chips(*t)).collect();
    let eq = icm_equities(&chips, &paiements).map_err(|e| e.to_string())?;
    Ok(tapis.into_iter().zip(eq).collect())
}

/// Le calculateur ICM, en direct.
pub fn icm(_props: &str) -> Node {
    let tapis = use_state(String::from("5000, 3000, 2000, 1500"));
    let paiements = use_state(String::from("50, 30, 20"));
    let resultat = move || calculer_icm(&tapis.get(), &paiements.get());
    div()
        .class("tool card")
        .child(
            div()
                .class("row-2")
                .child(
                    div()
                        .class("field")
                        .child(label().attr("for", "icm-tapis").text("Tapis (séparés par des virgules)"))
                        .child(input().id("icm-tapis").attr("type", "text").attr("inputmode", "numeric").bind_value(tapis)),
                )
                .child(
                    div()
                        .class("field")
                        .child(label().attr("for", "icm-paie").text("Paiements (% ou montants)"))
                        .child(input().id("icm-paie").attr("type", "text").attr("inputmode", "decimal").bind_value(paiements)),
                ),
        )
        .child(div().class("icm-table").children_dyn(move || match resultat() {
            Ok(lignes) => {
                let total_tapis: u64 = lignes.iter().map(|l| l.0).sum::<u64>().max(1);
                let total_eq: f64 = lignes.iter().map(|l| l.1).sum::<f64>().max(f64::MIN_POSITIVE);
                lignes
                    .into_iter()
                    .enumerate()
                    .map(|(i, (t, e))| {
                        let part_tapis = t as f64 / total_tapis as f64;
                        let part_eq = e / total_eq;
                        Node::from(
                            div()
                                .class("icm-row")
                                .child(span().class("icm-seat").text(format!("J{}", i + 1)))
                                .child(span().class("icm-stack").text(format!("{t} jetons")))
                                .child(
                                    div()
                                        .class("icm-bars")
                                        .child(div().class("icm-chips").attr("style", format!("--v:{part_tapis:.4}")))
                                        .child(div().class("icm-money").attr("style", format!("--v:{part_eq:.4}"))),
                                )
                                .child(span().class("icm-val").text(format!("{:.2}", e).replace('.', ",")))
                                .child(span().class(if part_eq < part_tapis { "icm-diff neg" } else { "icm-diff" }).text(format!(
                                    "{:+.1} pt",
                                    (part_eq - part_tapis) * 100.0
                                ).replace('.', ","))),
                        )
                    })
                    .collect()
            }
            Err(e) => vec![Node::from(p().class("tool-info").text(format!("⚠️ {e}")))],
        }))
        .child(p().class("fine").text(
            "Barre claire : part des jetons · barre colorée : part de l'argent (ICM exact, Malmuth-Harville). L'écart montre la pression ICM.",
        ))
        .into()
}

/// La pluie de jetons : des jetons qui tombent, rebondissent et s'empilent sur le tapis,
/// avec la physique de `paquets::three::Physics`.
pub fn jetons(_props: &str) -> Node {
    let l = Liens::default();
    let compte = use_state(0u32);
    div()
        .class("pluie")
        .child(
            canvas()
                .class("pluie-canvas")
                .attr("role", "img")
                .attr(
                    "aria-label",
                    "Des jetons 3D tombent et s'empilent sur le tapis",
                )
                .use_action(move |canvas| lancer(canvas, Scene3d::Jetons, compte, l)),
        )
        .child(
            div()
                .class("pluie-bar")
                .child(
                    button()
                        .class("btn btn-primary magnetic")
                        .attr("type", "button")
                        .on_click(move |_| l.lacher.update(|n| *n += 15))
                        .text("Faire tapis (+15 jetons)"),
                )
                .child(
                    button()
                        .class("btn btn-ghost")
                        .attr("type", "button")
                        .on_click(move |_| l.secousse.update(|n| *n += 1))
                        .text("Secouer la table"),
                )
                .child(
                    span()
                        .class("pluie-count")
                        .text_dyn(move || match compte.get() {
                            0 => "Lâchez des jetons".into(),
                            1 => "1 jeton sur le tapis".into(),
                            n => format!("{n} jetons sur le tapis"),
                        }),
                ),
        )
        .into()
}

/// Le point d'entrée du navigateur : rend chaque îlot de la page interactif.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    paquets::hydrate(&ISLANDS);
}
