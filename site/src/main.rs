//! Le site de STRATYX — le serveur : les pages (rendues sur le serveur, avec leurs balises
//! SEO), la liste d'attente et le formulaire de contact, la PWA, le plan du site.
//!
//!     cargo run        # http://localhost:3000 (PORT en production)
//!
//! Les scènes 3D, les démos du moteur poker et les mouvements de la page sont des îlots
//! (src/lib.rs), compilés en WebAssembly dans `pkg/` : voir le README.

use paquets::prelude::*;

mod contenu;
mod icones;

use contenu::*;
use icones::icone;

// ————————————————————————————————————————————————————————————————————————————
// Données
// ————————————————————————————————————————————————————————————————————————————

/// Une inscription à la liste d'attente.
#[model]
struct Inscrit {
    id: i64,
    email: String,
    profil: String,
    inscrit_le: i64,
}

/// Un message envoyé depuis la page de contact.
#[model]
struct Message {
    id: i64,
    nom: String,
    email: String,
    sujet: String,
    contenu: String,
    recu_le: i64,
}

#[data]
struct FormulaireListe {
    email: String,
    #[serde(default)]
    profil: String,
    /// Un champ caché que seuls les robots remplissent.
    #[serde(default)]
    site_web: String,
}

#[data]
struct FormulaireContact {
    nom: String,
    email: String,
    #[serde(default)]
    sujet: String,
    contenu: String,
    #[serde(default)]
    site_web: String,
}

fn maintenant() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn email_valide(email: &str) -> bool {
    let email = email.trim();
    let Some((avant, apres)) = email.split_once('@') else {
        return false;
    };
    !avant.is_empty()
        && apres.contains('.')
        && !apres.starts_with('.')
        && !apres.ends_with('.')
        && email.len() <= 254
        && !email.contains(char::is_whitespace)
}

fn borne(texte: &str, max: usize) -> String {
    texte.trim().chars().take(max).collect()
}

// ————————————————————————————————————————————————————————————————————————————
// La mise en page commune
// ————————————————————————————————————————————————————————————————————————————

/// L'adresse publique du site, pour les URL canoniques et Open Graph.
fn site_url() -> String {
    std::env::var("SITE_URL")
        .unwrap_or_else(|_| "http://localhost:3000".into())
        .trim_end_matches('/')
        .to_owned()
}

/// Une page du site : `chemin` marque le lien actif du menu.
fn page(chemin: &str, titre: &str, description: &str, contenu: Vec<Node>) -> Page {
    let base = site_url();
    let json_ld = format!(
        r#"{{"@context":"https://schema.org","@type":"SoftwareApplication","name":"{NOM}","applicationCategory":"GameApplication","operatingSystem":"Windows, macOS, Linux, iOS, Android","url":"{base}","description":"Plateforme d'analyse de Texas Hold'em écrite en Rust : capture, retranscription, analyse stratégique, profils et coaching."}}"#
    );
    Page::new(titre)
        .lang("fr")
        .description(description)
        .seo(
            Seo::new()
                .site_name(NOM)
                .title_template(format!("%s · {NOM}"))
                .base_url(base)
                .canonical(chemin)
                .image("/og.png")
                .locale("fr_FR")
                .json_ld(json_ld),
        )
        .plain()
        .stylesheet(
            "https://fonts.googleapis.com/css2?family=Space+Grotesk:wght@400;500;600;700&family=Inter:wght@400;500;600&display=swap",
        )
        .stylesheet("/style.css")
        .head(concat!(
            r#"<link rel="icon" href="/favicon.svg" type="image/svg+xml"/>"#,
            r#"<link rel="apple-touch-icon" href="/icons/apple-touch-icon.png"/>"#,
            r#"<meta name="color-scheme" content="dark"/>"#,
        ))
        .view_transitions()
        .prefetch(Prefetch::Hover)
        .body(curseur())
        .body(island("mouvement", "", stratyx_site::mouvement))
        .body(entete(chemin))
        .body(html::main().id("contenu").class("page").children(contenu))
        .body(pied_de_page())
}

fn curseur() -> Element {
    div()
        .class("cursor")
        .attr("aria-hidden", "true")
        .child(div().class("cursor-dot"))
        .child(div().class("cursor-ring"))
}

fn logo() -> Element {
    a().class("logo")
        .href("/")
        .attr("aria-label", "STRATYX, accueil")
        .child(span().class("logo-mark").html(icone("logo")))
        .child(span().class("logo-text").text("STRATYX"))
}

fn entete(chemin: &str) -> Element {
    let liens = MENU.iter().map(|(href, label)| {
        let actif = chemin.starts_with(href);
        a().href(*href)
            .class("nav-link")
            .class_if("active", move || actif)
            .attr("data-text", *label)
            .child(span().text(*label))
    });
    header()
        .class("site-header")
        .child(a().class("skip").href("#contenu").text("Aller au contenu"))
        .child(div().class("progress").attr("aria-hidden", "true"))
        .child(
            div()
                .class("nav-shell")
                .child(logo())
                .child(
                    nav()
                        .class("nav")
                        .id("menu")
                        .attr("aria-label", "Navigation principale")
                        .children(liens)
                        .child(
                            a().class("btn btn-primary btn-sm magnetic nav-cta")
                                .href("#liste-attente")
                                .text("Accès anticipé")
                                .child(span().class("arrow").html(icone("fleche"))),
                        ),
                )
                .child(
                    button()
                        .class("burger")
                        .attr("type", "button")
                        .attr("aria-label", "Ouvrir le menu")
                        .attr("aria-expanded", "false")
                        .attr("aria-controls", "menu")
                        .child(span())
                        .child(span())
                        .child(span()),
                ),
        )
}

fn champ_cache() -> Element {
    input()
        .class("hp")
        .attr("type", "text")
        .attr("name", "site_web")
        .attr("tabindex", "-1")
        .attr("autocomplete", "off")
        .attr("aria-hidden", "true")
}

fn pied_de_page() -> Element {
    let colonne = |titre: &'static str, liens: &'static [(&'static str, &'static str)]| {
        div().class("footer-col").child(h4().text(titre)).child(
            ul().children(
                liens
                    .iter()
                    .map(|(href, label)| li().child(a().href(*href).class("u-link").text(*label))),
            ),
        )
    };
    footer()
        .class("site-footer")
        .child(div().class("footer-glow").attr("aria-hidden", "true"))
        // La liste d'attente et l'appel à l'action.
        .child(
            div()
                .class("container footer-top")
                .attr("data-reveal", "up")
                .child(
                    div()
                        .class("newsletter")
                        .id("liste-attente")
                        .child(span().class("eyebrow").text("Accès anticipé"))
                        .child(h3().text("Soyez parmi les premiers à analyser vos mains."))
                        .child(p().class("muted").text(
                            "Un e-mail à l'ouverture de chaque étape, et une invitation à l'accès anticipé. Rien d'autre.",
                        ))
                        .child(
                            form()
                                .class("newsletter-form")
                                .attr("method", "post")
                                .attr("action", "/liste-attente")
                                .child(label().class("sr-only").attr("for", "la-email").text("Adresse e-mail"))
                                .child(
                                    input()
                                        .id("la-email")
                                        .attr("type", "email")
                                        .attr("name", "email")
                                        .attr("autocomplete", "email")
                                        .attr("placeholder", "vous@exemple.fr")
                                        .attr("required", ""),
                                )
                                .child(label().class("sr-only").attr("for", "la-profil").text("Votre profil"))
                                .child(
                                    select()
                                        .id("la-profil")
                                        .attr("name", "profil")
                                        .children(PROFILS.map(|p| option().attr("value", p).text(p))),
                                )
                                .child(champ_cache())
                                .child(
                                    button()
                                        .class("btn btn-primary magnetic")
                                        .attr("type", "submit")
                                        .text("Je m'inscris")
                                        .child(span().class("arrow").html(icone("fleche"))),
                                ),
                        )
                        .child(p().class("fine").text(
                            "Désinscription en un clic. Voir la politique de confidentialité.",
                        )),
                )
                .child(
                    div()
                        .class("footer-cta")
                        .child(p().class("big").text("Testez le moteur maintenant"))
                        .child(
                            a().class("cta-orb magnetic")
                                .href("/laboratoire")
                                .attr("aria-label", "Ouvrir le laboratoire")
                                .child(span().text("Laboratoire"))
                                .child(span().class("orb-ring").attr("aria-hidden", "true")),
                        ),
                ),
        )
        // Les colonnes.
        .child(
            div()
                .class("container footer-grid")
                .child(
                    div()
                        .class("footer-brand")
                        .child(logo())
                        .child(p().class("muted").text(
                            "La plateforme d'analyse de Texas Hold'em, écrite entièrement en Rust : capture, retranscription, analyse stratégique, profils et coaching.",
                        ))
                        .child(
                            div().class("socials").child(
                                a().class("social magnetic")
                                    .href(DEPOT)
                                    .attr("aria-label", "STRATYX sur GitHub")
                                    .attr("rel", "noopener noreferrer")
                                    .attr("target", "_blank")
                                    .html(icone("github")),
                            ),
                        )
                        .child(
                            div()
                                .class("status")
                                .child(span().class("pulse").attr("aria-hidden", "true"))
                                .text("Phase 1 en cours · moteur poker livré"),
                        ),
                )
                .child(colonne("Produit", PIED_PRODUIT))
                .child(colonne("Ressources", PIED_RESSOURCES))
                .child(colonne("Projet", PIED_PROJET))
                .child(
                    div()
                        .class("footer-col footer-contact")
                        .child(h4().text("Engagements"))
                        .child(
                            address()
                                .child(p().child(span().class("ico").html(icone("bouclier"))).child(span().text("Aucune assistance en direct")))
                                .child(p().child(span().class("ico").html(icone("rouage"))).child(span().text("100 % Rust, audité à chaque push")))
                                .child(p().child(span().class("ico").html(icone("cadenas"))).child(span().text("Données protégées, RGPD"))),
                        )
                        .child(
                            button()
                                .class("btn btn-ghost btn-sm install-app")
                                .attr("type", "button")
                                .attr("hidden", "")
                                .child(span().class("ico").html(icone("telecharger")))
                                .text("Installer l'application"),
                        ),
                ),
        )
        // Le jeu responsable.
        .child(
            div().class("container").child(
                div()
                    .class("responsible")
                    .child(span().class("age").text("18+"))
                    .child(p().text(
                        "Jouer comporte des risques : endettement, dépendance… Appelez le 09 74 75 13 13 (Joueurs Info Service, appel non surtaxé). Les jeux d'argent sont interdits aux mineurs.",
                    ))
                    .child(a().class("u-link").href("/fair-play").text("Fair-play")),
            ),
        )
        // Le grand nom.
        .child(div().class("footer-word").attr("aria-hidden", "true").child(span().text("STRATYX")))
        // La ligne légale.
        .child(
            div()
                .class("container footer-bottom")
                .child(p().text(format!("© {ANNEE} {NOM} · Tous droits réservés.")))
                .child(
                    nav()
                        .class("legal")
                        .attr("aria-label", "Informations légales")
                        .child(a().class("u-link").href("/mentions-legales").text("Mentions légales"))
                        .child(a().class("u-link").href("/confidentialite").text("Confidentialité"))
                        .child(a().class("u-link").href("/plan-du-site").text("Plan du site"))
                        .child(a().class("u-link").href("/sitemap.xml").text("sitemap.xml")),
                )
                .child(
                    p().class("made")
                        .text("Site 100 % Rust, fait avec ")
                        .child(a().class("u-link").href("https://github.com/maxlestage/Paquets").text("Paquets 📦"))
                        .text(" · WebAssembly"),
                )
                .child(
                    button()
                        .class("to-top magnetic")
                        .attr("type", "button")
                        .attr("aria-label", "Revenir en haut")
                        .html(icone("haut")),
                ),
        )
}

// ————————————————————————————————————————————————————————————————————————————
// Briques des pages
// ————————————————————————————————————————————————————————————————————————————

/// Découpe un titre en mots, pour qu'ils apparaissent l'un après l'autre. Les mots entre
/// `*` prennent le dégradé.
fn mots(el: Element, texte: &str) -> Element {
    let mut el = el.class("split").attr("aria-label", texte.replace('*', ""));
    for (i, mot) in texte.split(' ').enumerate() {
        let (grad, reste) = match mot.strip_prefix('*').and_then(|m| m.split_once('*')) {
            Some((g, r)) => (g.to_owned(), r.to_owned()),
            None => (String::new(), mot.to_owned()),
        };
        let mut wi = span().class("wi").attr("style", format!("--i:{i}"));
        if !grad.is_empty() {
            wi = wi.child(span().class("grad").text(grad));
        }
        if !reste.is_empty() {
            wi = wi.text(reste);
        }
        el = el
            .child(span().class("w").attr("aria-hidden", "true").child(wi))
            .text(" ");
    }
    el
}

fn titre_section(eyebrow: &str, titre: &str, chapeau: &str) -> Element {
    div()
        .class("section-head")
        .child(
            span()
                .class("eyebrow")
                .attr("data-reveal", "up")
                .text(eyebrow.to_owned()),
        )
        .child(mots(h2(), titre))
        .child(
            p().class("lead")
                .attr("data-reveal", "up")
                .attr("style", "--d:200ms")
                .text(chapeau.to_owned()),
        )
}

/// L'en-tête des pages intérieures.
fn bandeau(eyebrow: &str, titre: &str, chapeau: &str) -> Node {
    section()
        .class("page-hero")
        .child(div().class("aurora").attr("aria-hidden", "true"))
        .child(div().class("grid-floor").attr("aria-hidden", "true"))
        .child(cartes_volantes())
        .child(
            div()
                .class("container")
                .child(
                    span()
                        .class("eyebrow")
                        .attr("data-reveal", "up")
                        .text(eyebrow.to_owned()),
                )
                .child(mots(h1(), titre))
                .child(
                    p().class("lead")
                        .attr("data-reveal", "up")
                        .attr("style", "--d:250ms")
                        .text(chapeau.to_owned()),
                ),
        )
        .into()
}

/// Des cartes en CSS 3D qui flottent et se retournent.
fn cartes_volantes() -> Element {
    div()
        .class("float-cards")
        .attr("aria-hidden", "true")
        .children(
            [
                ("A", "♠", false),
                ("K", "♥", true),
                ("Q", "♦", true),
                ("J", "♣", false),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, (r, s, rouge))| {
                div().class(format!("fcard f{i}")).child(
                    div()
                        .class("fcard-inner")
                        .child(
                            div()
                                .class(if rouge {
                                    "fcard-face red"
                                } else {
                                    "fcard-face"
                                })
                                .child(span().class("pc-r").text(r))
                                .child(span().class("pc-big").text(s)),
                        )
                        .child(div().class("fcard-back")),
                )
            }),
        )
}

fn carte_fonction(i: usize, f: &Fonction) -> Element {
    article()
        .class("card service tilt glow")
        .attr("data-reveal", "up")
        .attr("style", format!("--d:{}ms", i * 90))
        .child(div().class("card-ico").html(icone(f.icone)))
        .child(h3().text(f.titre))
        .child(p().class("muted").text(f.texte))
        .child(
            ul().class("tags")
                .children(f.tags.iter().map(|t| li().text(*t))),
        )
}

fn chiffres() -> Element {
    div()
        .class("stats")
        .children(CHIFFRES.iter().enumerate().map(|(i, (n, suffixe, label))| {
            div()
                .class("stat")
                .attr("data-reveal", "up")
                .attr("style", format!("--d:{}ms", i * 100))
                .child(
                    p().class("stat-num")
                        .child(span().attr("data-count", n.to_string()).text("0"))
                        .text(*suffixe),
                )
                .child(p().class("muted").text(*label))
        }))
}

fn defilant(items: &[&'static str], inverse: bool) -> Element {
    let rangee = || {
        items.iter().map(|nom| {
            span()
                .class("marquee-item")
                .child(span().class("star").text("♠"))
                .text(*nom)
        })
    };
    div()
        .class(if inverse {
            "marquee reverse"
        } else {
            "marquee"
        })
        .attr("aria-hidden", "true")
        .child(
            div()
                .class("marquee-track")
                .children(rangee())
                .children(rangee()),
        )
}

fn carte_ia(i: usize, ia: &Ia, detail: bool) -> Element {
    article()
        .class(if i == 0 {
            "ia-card tilt glow ia-a"
        } else {
            "ia-card tilt glow ia-b"
        })
        .id(ia.id)
        .attr("data-reveal", if i == 0 { "left" } else { "right" })
        .child(
            div()
                .class("ia-orb")
                .attr("aria-hidden", "true")
                .child(span())
                .child(span())
                .child(span()),
        )
        .child(
            span()
                .class("ia-tag")
                .text(if i == 0 { "3 joueurs" } else { "6 joueurs" }),
        )
        .child(h3().text(ia.nom))
        .child(p().class("muted").text(ia.accroche))
        .child(
            ul().class("checks")
                .children(ia.points.iter().map(|p| li().text(*p))),
        )
        .child(if detail {
            p().class("fine")
                .child(code().text(ia.crate_rs))
                .text(" · moteur commun stratyx-poker")
        } else {
            p().class("alert-none")
        })
}

fn appel_final() -> Node {
    section()
        .class("section")
        .child(
            div().class("container").child(
                div()
                    .class("cta-band")
                    .attr("data-reveal", "zoom")
                    .child(div().class("cta-orbits").attr("aria-hidden", "true").children(
                        (0..3).map(|i| div().class(format!("orbit o{i}")).child(span())),
                    ))
                    .child(span().class("eyebrow").text("Accès anticipé"))
                    .child(mots(h2(), "Jouez *mieux*, pas plus"))
                    .child(p().class("lead").text(
                        "Rejoignez la liste d'attente : vous serez prévenu à chaque étape et invité parmi les premiers.",
                    ))
                    .child(
                        div()
                            .class("actions")
                            .child(
                                a().class("btn btn-primary btn-lg magnetic")
                                    .href("#liste-attente")
                                    .text("Rejoindre la liste d'attente")
                                    .child(span().class("arrow").html(icone("fleche"))),
                            )
                            .child(
                                a().class("btn btn-ghost btn-lg magnetic")
                                    .href("/laboratoire")
                                    .text("Essayer le moteur"),
                            ),
                    ),
            ),
        )
        .into()
}

// ————————————————————————————————————————————————————————————————————————————
// Les pages
// ————————————————————————————————————————————————————————————————————————————

async fn accueil(_req: Request) -> Page {
    let hero = section()
        .class("hero")
        .child(div().class("aurora").attr("aria-hidden", "true"))
        .child(island("hero3d", "", stratyx_site::hero3d))
        .child(
            div()
                .class("container hero-inner")
                .child(
                    a().class("pill")
                        .href("/laboratoire")
                        .attr("data-reveal", "down")
                        .child(span().class("pill-tag").text("Phase 1"))
                        .text("Le moteur poker est livré : essayez-le")
                        .child(span().class("arrow").html(icone("fleche"))),
                )
                .child(mots(h1().class("hero-title"), "Analysez chaque main. *Progressez* à chaque session."))
                .child(
                    p().class("lead hero-lead")
                        .attr("data-reveal", "up")
                        .attr("style", "--d:500ms")
                        .text(
                            "STRATYX retranscrit vos parties de Texas Hold'em, analyse chaque décision avec deux IA spécialisées et vous entraîne sur vos vraies fuites. Écrit entièrement en Rust.",
                        ),
                )
                .child(
                    div()
                        .class("actions")
                        .attr("data-reveal", "up")
                        .attr("style", "--d:650ms")
                        .child(
                            a().class("btn btn-primary btn-lg magnetic")
                                .href("#liste-attente")
                                .text("Accès anticipé")
                                .child(span().class("arrow").html(icone("fleche"))),
                        )
                        .child(
                            a().class("btn btn-ghost btn-lg magnetic")
                                .href("#demo")
                                .child(span().class("play").html(icone("play")))
                                .text("Voir la démo 3D"),
                        ),
                )
                .child(
                    ul().class("hero-proof")
                        .attr("data-reveal", "up")
                        .attr("style", "--d:800ms")
                        .child(li().child(strong().text("NL · PL · FL")))
                        .child(li().child(strong().text("2 à 10")).text(" joueurs"))
                        .child(li().child(strong().text("100 %")).text(" Rust")),
                ),
        )
        .child(
            a().class("scroll-hint")
                .href("#fonctionnalites")
                .attr("aria-label", "Défiler vers la suite")
                .child(span().class("mouse").child(span())),
        );

    let bandeau_formats = section()
        .class("logos")
        .attr("aria-label", "Formats couverts")
        .child(p().class("logos-title").text("Tout le Texas Hold'em"))
        .child(defilant(
            &[
                "No-limit",
                "Pot-limit",
                "Limite fixe",
                "Cash game",
                "Tournois",
                "Sit & go",
                "Expresso",
                "Heads-up",
                "6-Max",
                "Table pleine",
            ],
            false,
        ));

    let fonctions = section().class("section").id("fonctionnalites").child(
        div()
            .class("container")
            .child(titre_section(
                "Fonctionnalités",
                "De la vidéo à la *décision* juste",
                "Capture, retranscription, analyse, profils, statistiques et coaching : toute la chaîne, dans un seul logiciel.",
            ))
            .child(div().class("grid-3").children(FONCTIONS.iter().enumerate().map(|(i, f)| carte_fonction(i, f)))),
    );

    let demo = section().class("section section-alt").id("demo").child(
        div()
            .class("container")
            .child(titre_section(
                "Démo en direct",
                "Distribuez, le moteur *analyse*",
                "Une table en 3D et une vraie main : à chaque carte, le moteur STRATYX, compilé en WebAssembly, évalue votre main et calcule votre équité. Changez de format, tournez autour de la table.",
            ))
            .child(div().attr("data-reveal", "zoom").child(island("table3d", "", stratyx_site::table3d))),
    );

    let ias = section().class("section").id("ia").child(
        div()
            .class("container")
            .child(titre_section(
                "Deux IA spécialisées",
                "Une IA par *format*, un moteur commun",
                "Chacune a ses archives, ses entraînements et ses évaluations. Les autres formats sont analysés par le moteur commun.",
            ))
            .child(div().class("grid-2").children(IAS.iter().enumerate().map(|(i, ia)| carte_ia(i, ia, false)))),
    );

    let methode = section().class("section section-alt").child(
        div()
            .class("container")
            .child(titre_section(
                "Comment ça marche",
                "Quatre étapes, *après* la partie",
                "L'analyse et l'entraînement se font en dehors du jeu, jamais pendant une partie réelle.",
            ))
            .child(
                ol().class("timeline")
                    .child(div().class("timeline-line").attr("aria-hidden", "true").child(span()))
                    .children(ETAPES.iter().enumerate().map(|(i, (titre, texte))| {
                        li().class("step")
                            .attr("data-reveal", if i % 2 == 0 { "left" } else { "right" })
                            .child(span().class("step-num").text(format!("0{}", i + 1)))
                            .child(
                                div()
                                    .class("step-card glow")
                                    .child(h3().text(*titre))
                                    .child(p().class("muted").text(*texte)),
                            )
                    })),
            ),
    );

    let moteur = section().class("section").child(
        div()
            .class("container")
            .child(titre_section(
                "Le moteur",
                "Des fondations *vérifiées*",
                "Le moteur commun est livré et testé : chaque chiffre ci-dessous se vérifie dans le dépôt.",
            ))
            .child(chiffres()),
    );

    page(
        "/",
        "Analyse de Texas Hold'em en Rust",
        "STRATYX retranscrit vos parties de Texas Hold'em, analyse chaque décision avec deux IA spécialisées (Expresso 3-Max, Tournois 6-Max) et vous entraîne. 100 % Rust.",
        vec![
            hero.into(),
            bandeau_formats.into(),
            fonctions.into(),
            demo.into(),
            ias.into(),
            methode.into(),
            moteur.into(),
            appel_final(),
        ],
    )
}

async fn fonctionnalites(_req: Request) -> Page {
    let grille = section().class("section").child(
        div().class("container").child(
            div().class("grid-3").children(
                FONCTIONS
                    .iter()
                    .enumerate()
                    .map(|(i, f)| carte_fonction(i, f)),
            ),
        ),
    );
    let formats =
        section().class("section section-alt").child(
            div()
                .class("container")
                .child(titre_section(
                    "Couverture",
                    "Tout le Texas Hold'em, *rien* d'autre",
                    "No-limit, pot-limit et limite fixe, de 2 à 10 joueurs, de UTG à BB.",
                ))
                .child(
                    div()
                        .class("grid-4")
                        .children(FORMATS_COUVERTS.iter().enumerate().map(
                            |(i, (titre, texte))| {
                                div()
                                    .class("card value glow tilt")
                                    .attr("data-reveal", "up")
                                    .attr("style", format!("--d:{}ms", i * 90))
                                    .child(div().class("card-ico").html(icone("cartes")))
                                    .child(h3().text(*titre))
                                    .child(p().class("muted").text(*texte))
                            },
                        )),
                ),
        );
    let faq = section().class("section").id("faq").child(
        div()
            .class("container narrow")
            .child(titre_section(
                "FAQ",
                "Vos questions, nos *réponses*",
                "Ce que fait STRATYX, et ce qu'il ne fera jamais.",
            ))
            .child(
                div()
                    .class("faq")
                    .children(FAQ.iter().enumerate().map(|(i, (q, r))| {
                        details()
                            .class("faq-item")
                            .attr("data-reveal", "up")
                            .attr("style", format!("--d:{}ms", i * 70))
                            .child(
                                summary()
                                    .child(span().text(*q))
                                    .child(span().class("plus").attr("aria-hidden", "true")),
                            )
                            .child(div().class("faq-body").child(p().text(*r)))
                    })),
            ),
    );
    page(
        "/fonctionnalites",
        "Fonctionnalités",
        "Capture vidéo, retranscription validée par rejeu, analyse GTO et exploitante, profils d'adversaires, statistiques et coaching.",
        vec![
            bandeau(
                "Fonctionnalités",
                "Tout ce qu'il faut pour *progresser*",
                "Une chaîne complète, de la capture de votre session à l'exercice qui corrige votre fuite.",
            ),
            grille.into(),
            formats.into(),
            faq.into(),
            appel_final(),
        ],
    )
}

async fn les_ia(_req: Request) -> Page {
    let cartes = section().class("section").child(
        div().class("container").child(
            div()
                .class("grid-2")
                .children(IAS.iter().enumerate().map(|(i, ia)| carte_ia(i, ia, true))),
        ),
    );
    let entrainement = section().class("section section-alt").child(
        div()
            .class("container")
            .child(titre_section(
                "Entraînement",
                "Des archives *propres*, des simulations",
                "Avant tout entraînement, les archives sont nettoyées, dédupliquées et validées par rejeu ; la licence de chaque source est vérifiée. Les simulations complètent les archives.",
            ))
            .child(div().class("grid-4").children(
                [
                    ("liste", "Nettoyage", "Mains structurées, dédupliquées, validées par rejeu dans le moteur de règles."),
                    ("cadenas", "Licences", "Chaque source d'archives est vérifiée avant usage."),
                    ("puce", "CPU et GPU", "Entraînement et inférence avec Burn, en ndarray ou wgpu."),
                    ("cible", "Évaluations", "Chaque IA a ses propres évaluations, reproductibles à graine égale."),
                ]
                .into_iter()
                .enumerate()
                .map(|(i, (ico, titre, texte))| {
                    div()
                        .class("card value glow tilt")
                        .attr("data-reveal", "up")
                        .attr("style", format!("--d:{}ms", i * 90))
                        .child(div().class("card-ico").html(icone(ico)))
                        .child(h3().text(titre))
                        .child(p().class("muted").text(texte))
                }),
            )),
    );
    page(
        "/ia",
        "Les deux IA",
        "IA Expresso 3-Max et IA Tournois 6-Max : deux intelligences spécialisées, adossées au moteur de calcul commun de STRATYX.",
        vec![
            bandeau(
                "Les IA",
                "Deux *spécialistes*, un moteur commun",
                "L'IA Expresso 3-Max pour les formats rapides à trois, l'IA Tournois 6-Max pour les tournois à six.",
            ),
            cartes.into(),
            entrainement.into(),
            appel_final(),
        ],
    )
}

async fn laboratoire(_req: Request) -> Page {
    let outil =
        |id: &str, eyebrow: &str, titre: &str, chapeau: &str, ile: Node, alt: bool| -> Node {
            section()
                .class(if alt {
                    "section section-alt"
                } else {
                    "section"
                })
                .id(id.to_owned())
                .child(
                    div()
                        .class("container")
                        .child(titre_section(eyebrow, titre, chapeau))
                        .child(div().attr("data-reveal", "zoom").child(ile)),
                )
                .into()
        };
    page(
        "/laboratoire",
        "Laboratoire : le moteur en direct",
        "Calculateur d'équité, grille des ranges et calculateur ICM : le vrai moteur stratyx-poker, compilé en WebAssembly, dans votre navigateur.",
        vec![
            bandeau(
                "Laboratoire",
                "Le vrai moteur, dans votre *navigateur*",
                "Ces outils utilisent stratyx-poker, le moteur de STRATYX, compilé en WebAssembly. Les calculs se font sur votre appareil : rien n'est envoyé.",
            ),
            outil(
                "equite",
                "Équité",
                "Qui *gagne*, et combien de fois ?",
                "Deux ou trois mains ou ranges, un board facultatif : calcul exact quand c'est possible, Monte-Carlo reproductible sinon.",
                island("equite", "", stratyx_site::equite),
                false,
            ),
            outil(
                "ranges",
                "Ranges",
                "La grille des *169* mains",
                "Écrivez une range en syntaxe standard (22+, A2s+, KTo+, pondérations 99:0.5…) : la grille et le nombre de combinaisons suivent.",
                island("grille", "", stratyx_site::grille),
                true,
            ),
            outil(
                "icm",
                "ICM",
                "Des jetons à l'*argent*",
                "Le modèle ICM exact (Malmuth-Harville) : la valeur réelle de chaque tapis selon la structure de paiement.",
                island("icm", "", stratyx_site::icm),
                false,
            ),
            outil(
                "physique",
                "Récréation",
                "Faites *tapis*",
                "Masse, gravité, rebonds et collisions : chaque jeton est un corps physique simulé en Rust. Lâchez-en, secouez la table, tournez autour.",
                island("jetons", "", stratyx_site::jetons),
                true,
            ),
        ],
    )
}

async fn technologie(_req: Request) -> Page {
    let principes = section().class("section").child(
        div()
            .class("container")
            .child(titre_section(
                "Principes",
                "Rust, de bout en *bout*",
                "Aucune dépendance native tierce en C, C++, Python, Swift ou Kotlin. La règle est contrôlée automatiquement.",
            ))
            .child(div().class("grid-4").children(PRINCIPES.iter().enumerate().map(|(i, (ico, titre, texte))| {
                div()
                    .class("card value glow tilt")
                    .attr("data-reveal", "up")
                    .attr("style", format!("--d:{}ms", i * 90))
                    .child(div().class("card-ico").html(icone(ico)))
                    .child(h3().text(*titre))
                    .child(p().class("muted").text(*texte))
            }))),
    );
    let architecture = section().class("section section-alt").child(
        div()
            .class("container")
            .child(titre_section(
                "Architecture",
                "Un workspace, des *modules*",
                "Le moteur poker ne dépend d'aucun module d'entrée-sortie ; les dépendances vont dans un seul sens, sans cycle.",
            ))
            .child(div().class("modules").children(MODULES.iter().enumerate().map(|(i, (chemin, role))| {
                div()
                    .class(if i == 0 { "module core" } else { "module" })
                    .attr("data-reveal", "up")
                    .attr("style", format!("--d:{}ms", (i % 4) * 70))
                    .child(code().text(*chemin))
                    .child(p().class("muted").text(*role))
            })))
            .child(
                pre()
                    .class("terminal")
                    .attr("data-reveal", "up")
                    .child(code().text(
                        "$ cargo run --release -p stratyx-cli -- equity AhKh \"QQ+,AKo\" --board Qs7h2h\n$ cargo run --release -p stratyx-cli -- icm --stacks 5000,3000,2000 --payouts 50,30,20\n$ cargo deny check   # règle « 100 % Rust »",
                    )),
            ),
    );
    let site = section().class("section").child(
        div()
            .class("container narrow")
            .child(titre_section(
                "Ce site",
                "Lui aussi, en *Rust*",
                "Pages rendues par Paquets sur le serveur, îlots et scènes 3D en WebAssembly (paquets::three), animations pilotées par Rust, moteur stratyx-poker dans le navigateur. Aucune ligne de JavaScript écrite à la main.",
            )),
    );
    page(
        "/technologie",
        "Technologie : 100 % Rust",
        "STRATYX est écrit entièrement en Rust : workspace Cargo, audit cargo-deny bloquant, calculs parallèles reproductibles, IA avec Burn sur CPU et GPU.",
        vec![
            bandeau(
                "Technologie",
                "*100 %* Rust, contrôlé à chaque push",
                "Moteurs, IA, interfaces, serveur et outils : tout est écrit en Rust, et un audit le vérifie automatiquement.",
            ),
            principes.into(),
            architecture.into(),
            site.into(),
            appel_final(),
        ],
    )
}

async fn feuille_de_route(_req: Request) -> Page {
    let phases =
        section()
            .class("section")
            .child(
                div()
                    .class("container narrow")
                    .child(ol().class("roadmap").children(PHASES.iter().map(
                        |(n, titre, statut, detail, avance)| {
                            let classe = match *statut {
                                "En cours" => "phase now",
                                "Commencée" => "phase started",
                                _ => "phase",
                            };
                            li().class(classe)
                                .attr("data-reveal", "left")
                                .child(span().class("phase-num").text(format!("{n}")))
                                .child(
                                    div()
                                        .class("phase-card glow")
                                        .child(
                                            p().class("phase-head")
                                                .child(h3().text(*titre))
                                                .child(span().class("phase-status").text(*statut)),
                                        )
                                        .child(p().class("muted").text(*detail))
                                        .child(div().class("phase-bar").child(
                                            span().attr("style", format!("--v:{avance:.2}")),
                                        )),
                                )
                        },
                    ))),
            );
    page(
        "/feuille-de-route",
        "Feuille de route",
        "Les huit phases de STRATYX : du moteur poker, déjà livré, aux applications bureau et mobile.",
        vec![
            bandeau(
                "Feuille de route",
                "Livré par *étapes*",
                "Huit phases, de la fondation au produit complet. La phase 1 est en cours : le moteur poker est livré et testé.",
            ),
            phases.into(),
            appel_final(),
        ],
    )
}

async fn fair_play(_req: Request) -> Page {
    let engagements = section().class("section").child(
        div()
            .class("container")
            .child(
                div()
                    .class("grid-2")
                    .children(FAIR_PLAY.iter().enumerate().map(|(i, (titre, texte))| {
                        div()
                            .class("card value glow")
                            .attr("data-reveal", "up")
                            .attr("style", format!("--d:{}ms", i * 90))
                            .child(
                                div()
                                    .class("card-ico")
                                    .html(icone(["bouclier", "cartes", "cadenas", "coeur"][i])),
                            )
                            .child(h3().text(*titre))
                            .child(p().class("muted").text(*texte))
                    })),
            ),
    );
    page(
        "/fair-play",
        "Fair-play et jeu responsable",
        "STRATYX n'apporte aucune aide pendant une partie réelle, respecte les salles et vos données. Jouer comporte des risques : 09 74 75 13 13.",
        vec![
            bandeau(
                "Fair-play",
                "Progresser, *jamais* tricher",
                "STRATYX est un outil d'analyse après la partie et d'entraînement. Ses règles sont écrites dans son cahier des charges.",
            ),
            engagements.into(),
        ],
    )
}

async fn contact(req: Request) -> Page {
    let erreur = req.query("erreur").is_some();
    let formulaire = form()
        .class("contact-form card")
        .attr("method", "post")
        .attr("action", "/contact")
        .attr("data-reveal", "up")
        .child(if erreur {
            p().class("alert").attr("role", "alert").text(
                "Merci de vérifier votre nom, votre e-mail et votre message (20 caractères minimum).",
            )
        } else {
            p().class("alert-none")
        })
        .child(
            div()
                .class("row-2")
                .child(champ("nom", "Nom", "text", "Camille Martin", true, "name"))
                .child(champ("email", "E-mail", "email", "camille@exemple.fr", true, "email")),
        )
        .child(
            div()
                .class("field")
                .child(label().attr("for", "sujet").text("Sujet"))
                .child(select().id("sujet").attr("name", "sujet").children(
                    ["Accès anticipé", "Partenariat ou coaching", "Presse", "Question technique", "Autre"]
                        .map(|s| option().attr("value", s).text(s)),
                )),
        )
        .child(
            div()
                .class("field")
                .child(label().attr("for", "contenu").text("Message"))
                .child(
                    textarea()
                        .id("contenu")
                        .attr("name", "contenu")
                        .attr("rows", "6")
                        .attr("minlength", "20")
                        .attr("maxlength", "4000")
                        .attr("required", "")
                        .attr("placeholder", "Votre format de jeu, ce que vous attendez de STRATYX…"),
                ),
        )
        .child(champ_cache())
        .child(
            div()
                .class("form-foot")
                .child(p().class("fine").text(
                    "Vos données servent uniquement à vous répondre. Voir la politique de confidentialité.",
                ))
                .child(
                    button()
                        .class("btn btn-primary btn-lg magnetic")
                        .attr("type", "submit")
                        .text("Envoyer")
                        .child(span().class("arrow").html(icone("fleche"))),
                ),
        );
    let infos = aside()
        .class("contact-aside")
        .child(
            div()
                .class("card radar-card")
                .attr("data-reveal", "up")
                .child(
                    div()
                        .class("radar")
                        .attr("aria-hidden", "true")
                        .child(span().class("sweep"))
                        .child(span().class("blip")),
                )
                .child(h3().text("Le projet"))
                .child(p().class("muted").text(
                    "STRATYX est développé en Rust, en public. Suivez l'avancement sur GitHub.",
                ))
                .child(
                    p().child(
                        a().class("u-link")
                            .href(DEPOT)
                            .text("github.com/maxlestage/stratyx"),
                    ),
                ),
        )
        .child(
            div()
                .class("card")
                .attr("data-reveal", "up")
                .attr("style", "--d:120ms")
                .child(h3().text("Ensuite"))
                .child(
                    ol().class("next-steps").children(
                        [
                            "Votre message est enregistré et lu par l'équipe.",
                            "Nous vous répondons par e-mail.",
                            "Pour l'accès anticipé, inscrivez-vous aussi à la liste d'attente.",
                        ]
                        .map(|t| li().text(t)),
                    ),
                ),
        );
    page(
        "/contact",
        "Contact",
        "Une question sur STRATYX, un partenariat, la presse ? Écrivez-nous.",
        vec![
            bandeau(
                "Contact",
                "Parlons *poker*",
                "Accès anticipé, partenariat, coaching, presse ou question technique : quelques lignes suffisent.",
            ),
            section()
                .class("section")
                .child(
                    div()
                        .class("container contact-grid")
                        .child(formulaire)
                        .child(infos),
                )
                .into(),
        ],
    )
}

fn champ(
    nom: &str,
    libelle: &str,
    genre: &str,
    exemple: &str,
    requis: bool,
    auto: &str,
) -> Element {
    let mut entree = input()
        .id(nom.to_owned())
        .attr("name", nom.to_owned())
        .attr("type", genre.to_owned())
        .attr("placeholder", exemple.to_owned())
        .attr("autocomplete", auto.to_owned())
        .attr("maxlength", "200");
    if requis {
        entree = entree.attr("required", "");
    }
    div()
        .class("field")
        .child(label().attr("for", nom.to_owned()).text(libelle.to_owned()))
        .child(entree)
}

async fn envoyer_contact(req: Request) -> Result<Redirect> {
    let f: FormulaireContact = req.form().await?;
    if !f.site_web.is_empty() {
        return Ok(Redirect::see_other("/merci?type=contact"));
    }
    if f.nom.trim().is_empty() || !email_valide(&f.email) || f.contenu.trim().chars().count() < 20 {
        return Ok(Redirect::see_other("/contact?erreur=1#contenu"));
    }
    let mut message = Message {
        id: 0,
        nom: borne(&f.nom, 200),
        email: borne(&f.email, 254),
        sujet: borne(&f.sujet, 60),
        contenu: borne(&f.contenu, 4000),
        recu_le: maintenant(),
    };
    message.insert(req.db()).await?;
    Ok(Redirect::see_other("/merci?type=contact"))
}

async fn inscrire(req: Request) -> Result<Redirect> {
    let f: FormulaireListe = req.form().await?;
    if !f.site_web.is_empty() {
        return Ok(Redirect::see_other("/merci?type=liste"));
    }
    if !email_valide(&f.email) {
        return Ok(Redirect::see_other("/merci?type=erreur"));
    }
    let email = borne(&f.email, 254).to_lowercase();
    let profil = if PROFILS.contains(&f.profil.as_str()) {
        f.profil
    } else {
        String::new()
    };
    let deja = Inscrit::query()
        .filter(Inscrit::EMAIL.eq(email.clone()))
        .all(req.db())
        .await?;
    if deja.is_empty() {
        let mut inscrit = Inscrit {
            id: 0,
            email,
            profil,
            inscrit_le: maintenant(),
        };
        inscrit.insert(req.db()).await?;
    }
    Ok(Redirect::see_other("/merci?type=liste"))
}

async fn merci(req: Request) -> Page {
    let (titre, texte, ok) = match req.query("type").as_deref() {
        Some("liste") => (
            "Vous êtes sur la *liste* !",
            "Inscription confirmée. Vous serez prévenu à l'ouverture de chaque étape et invité à l'accès anticipé.",
            true,
        ),
        Some("erreur") => (
            "Adresse *invalide*",
            "Cette adresse e-mail ne semble pas valide. Vous pouvez réessayer depuis le pied de page.",
            false,
        ),
        _ => (
            "Message *reçu* !",
            "Merci ! Nous vous répondons par e-mail.",
            true,
        ),
    };
    page(
        "/merci",
        "Merci",
        "Merci.",
        vec![
            section()
                .class("page-hero thanks")
                .child(div().class("aurora").attr("aria-hidden", "true"))
                .child(
                    div()
                        .class("container center")
                        .child(
                            div()
                                .class(if ok { "check-anim" } else { "check-anim bad" })
                                .attr("aria-hidden", "true")
                                .html(icone(if ok { "check" } else { "croix" })),
                        )
                        .child(mots(h1(), titre))
                        .child(p().class("lead").attr("data-reveal", "up").text(texte))
                        .child(
                            div()
                                .class("actions center")
                                .attr("data-reveal", "up")
                                .child(
                                    a().class("btn btn-primary magnetic")
                                        .href("/")
                                        .text("Retour à l'accueil"),
                                )
                                .child(
                                    a().class("btn btn-ghost magnetic")
                                        .href("/laboratoire")
                                        .text("Essayer le moteur"),
                                ),
                        ),
                )
                .into(),
        ],
    )
    .seo(Seo::new().noindex(true))
}

fn page_texte(
    chemin: &str,
    titre: &str,
    description: &str,
    sections: &[(&'static str, &'static str)],
) -> Page {
    page(
        chemin,
        titre,
        description,
        vec![
            bandeau("Informations", titre, description),
            section()
                .class("section")
                .child(
                    div()
                        .class("container narrow prose")
                        .children(sections.iter().map(|(h, t)| {
                            div()
                                .attr("data-reveal", "up")
                                .child(h2().text(*h))
                                .children(t.split("\n\n").map(|para| p().text(para.to_owned())))
                        })),
                )
                .into(),
        ],
    )
}

async fn mentions(_req: Request) -> Page {
    page_texte(
        "/mentions-legales",
        "Mentions légales",
        "Éditeur, hébergement et propriété intellectuelle.",
        &MENTIONS,
    )
}

async fn confidentialite(_req: Request) -> Page {
    page_texte(
        "/confidentialite",
        "Politique de confidentialité",
        "Quelles données nous collectons, pourquoi, combien de temps, et vos droits.",
        &CONFIDENTIALITE,
    )
}

async fn plan(_req: Request) -> Page {
    page(
        "/plan-du-site",
        "Plan du site",
        "Toutes les pages du site STRATYX.",
        vec![
            bandeau(
                "Navigation",
                "Plan du *site*",
                "Toutes nos pages, en un coup d'œil.",
            ),
            section()
                .class("section")
                .child(
                    div()
                        .class("container narrow")
                        .child(ul().class("sitemap").children(PAGES.iter().map(
                            |(href, label)| {
                                li().attr("data-reveal", "left")
                                    .child(a().class("u-link").href(*href).text(*label))
                            },
                        ))),
                )
                .into(),
        ],
    )
}

async fn introuvable(_req: Request) -> Page {
    page(
        "/404",
        "Page introuvable",
        "Cette page s'est couchée.",
        vec![
            section()
                .class("page-hero lost")
                .child(div().class("aurora").attr("aria-hidden", "true"))
                .child(div().class("grid-floor").attr("aria-hidden", "true"))
                .child(
                    div()
                        .class("container center")
                        .child(p().class("glitch").attr("data-text", "404").text("404"))
                        .child(cartes_volantes())
                        .child(mots(h1(), "Cette page s'est *couchée*"))
                        .child(p().class("lead").text("Elle a quitté la table. Elle a peut-être été déplacée, ou n'a jamais existé."))
                        .child(
                            div()
                                .class("actions center")
                                .child(a().class("btn btn-primary magnetic").href("/").text("Retour à l'accueil"))
                                .child(a().class("btn btn-ghost magnetic").href("/laboratoire").text("Le laboratoire")),
                        ),
                )
                .into(),
        ],
    )
    .seo(Seo::new().noindex(true))
    .status(404)
}

async fn sitemap(_req: Request) -> Response {
    let base = site_url();
    let mut xml = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">"#,
    );
    for (href, _) in PAGES {
        xml.push_str(&format!("<url><loc>{base}{href}</loc></url>"));
    }
    xml.push_str("</urlset>");
    Response::new()
        .header("content-type", "application/xml; charset=utf-8")
        .send(xml)
}

async fn robots(_req: Request) -> String {
    format!(
        "User-agent: *\nAllow: /\nSitemap: {}/sitemap.xml\n",
        site_url()
    )
}

fn app() -> App {
    // Paquets calcule une fois les empreintes CSP de `view_transitions()` et `prefetch()` en
    // rendant une page vide ; on le fait au démarrage, avant toute page avec des îlots
    // (corrigé dans Paquets sur la branche claude/site-vitrine-3d).
    let _ = Page::new("")
        .view_transitions()
        .prefetch(Prefetch::Hover)
        .render();

    let mut app = App::new();
    app.get("/", accueil);
    app.get("/fonctionnalites", fonctionnalites);
    app.get("/ia", les_ia);
    app.get("/laboratoire", laboratoire);
    app.get("/technologie", technologie);
    app.get("/feuille-de-route", feuille_de_route);
    app.get("/fair-play", fair_play);
    app.get("/contact", contact);
    app.post("/contact", envoyer_contact);
    app.post("/liste-attente", inscrire);
    app.get("/merci", merci);
    app.get("/mentions-legales", mentions);
    app.get("/confidentialite", confidentialite);
    app.get("/plan-du-site", plan);
    app.get("/sitemap.xml", sitemap);
    app.get("/robots.txt", robots);
    app.model::<Inscrit>();
    app.model::<Message>();
    app.health("/health");
    app.pwa(
        Pwa::new(NOM)
            .short_name(NOM)
            .description("Analyse de Texas Hold'em en Rust : retranscription, analyse stratégique et coaching.")
            .lang("fr")
            .start_url("/")
            .display("standalone")
            .theme_color("#07100c")
            .background_color("#07100c")
            .categories(["games", "education", "sports"])
            .icon("/icons/icon-192.png", "192x192", "image/png")
            .icon("/icons/icon-512.png", "512x512", "image/png")
            .maskable_icon("/icons/icon-maskable-512.png", "512x512", "image/png"),
    );
    app.fallback(introuvable);
    app
}

fn main() -> std::io::Result<()> {
    app().run()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn chaque_page_repond() {
        let client = app().test().await;
        for (chemin, _) in PAGES {
            let res = client.get(chemin).await;
            assert_eq!(res.status(), 200, "{chemin}");
            let html = res.text();
            assert!(html.contains("<html lang=\"fr\""), "{chemin}");
            assert!(html.contains("site-footer"), "{chemin}");
            assert!(html.contains("data-island=\"mouvement\""), "{chemin}");
            assert!(html.contains("manifest.webmanifest"), "{chemin}");
        }
        assert_eq!(client.get("/nulle-part").await.status(), 404);
        assert_eq!(client.get("/manifest.webmanifest").await.status(), 200);
        assert_eq!(client.get("/sw.js").await.status(), 200);
        assert_eq!(client.get("/sitemap.xml").await.status(), 200);
        assert_eq!(client.get("/health").await.status(), 200);
    }

    #[tokio::test]
    async fn les_iles_sont_rendues() {
        let client = app().test().await;
        let accueil = client.get("/").await.text();
        assert!(accueil.contains("data-island=\"hero3d\""));
        assert!(accueil.contains("data-island=\"table3d\""));
        let labo = client.get("/laboratoire").await.text();
        for ile in ["equite", "grille", "icm", "jetons"] {
            assert!(labo.contains(&format!("data-island=\"{ile}\"")), "{ile}");
        }
    }

    #[test]
    fn emails() {
        assert!(email_valide("camille@exemple.fr"));
        assert!(!email_valide("camille"));
        assert!(!email_valide("camille@exemple"));
        assert!(!email_valide("a b@exemple.fr"));
    }
}
