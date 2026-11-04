//! Les icônes du site, en SVG (traits de 1,75 px, couleur du texte).

fn trait_svg(chemins: &str) -> String {
    format!(
        r#"<svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">{chemins}</svg>"#
    )
}

fn plein_svg(chemins: &str) -> String {
    format!(
        r#"<svg viewBox="0 0 24 24" width="24" height="24" fill="currentColor" aria-hidden="true" focusable="false">{chemins}</svg>"#
    )
}

/// L'icône `nom` (une icône vide si elle n'existe pas).
pub fn icone(nom: &str) -> String {
    match nom {
        "logo" => r##"<svg viewBox="0 0 40 40" width="34" height="34" aria-hidden="true" focusable="false"><defs><linearGradient id="lg" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#5be3a8"/><stop offset="1" stop-color="#f2c14e"/></linearGradient></defs><rect class="logo-orbit" x="7" y="7" width="26" height="26" rx="6" fill="none" stroke="url(#lg)" stroke-width="2" transform="rotate(45 20 20)"/><path d="M20 10.5c-2.6 3.4-7 5.6-7 9.6a3.6 3.6 0 0 0 6.1 2.6l-.9 4.8h3.6l-.9-4.8a3.6 3.6 0 0 0 6.1-2.6c0-4-4.4-6.2-7-9.6z" fill="url(#lg)"/><circle class="logo-moon" cx="33.5" cy="20" r="2.2" fill="#ff4d5e"/></svg>"##.into(),
        "fleche" => trait_svg(r#"<path d="M5 12h14"/><path d="m13 6 6 6-6 6"/>"#),
        "haut" => trait_svg(r#"<path d="M12 19V5"/><path d="m6 11 6-6 6 6"/>"#),
        "play" => plein_svg(r#"<path d="M8 5.5v13a1 1 0 0 0 1.5.86l10.5-6.5a1 1 0 0 0 0-1.72L9.5 4.64A1 1 0 0 0 8 5.5z"/>"#),
        "lieu" => trait_svg(r#"<path d="M12 21s-7-6.2-7-11.5A7 7 0 0 1 19 9.5C19 14.8 12 21 12 21z"/><circle cx="12" cy="9.5" r="2.5"/>"#),
        "mail" => trait_svg(r#"<rect x="3" y="5" width="18" height="14" rx="2.5"/><path d="m4 7 8 6 8-6"/>"#),
        "tel" => trait_svg(r#"<path d="M5 4h3.5l1.8 4.5-2.3 1.4a11 11 0 0 0 6.1 6.1l1.4-2.3L20 15.5V19a1.5 1.5 0 0 1-1.6 1.5A16 16 0 0 1 3.5 5.6 1.5 1.5 0 0 1 5 4z"/>"#),
        "horloge" => trait_svg(r#"<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>"#),
        "telecharger" => trait_svg(r#"<path d="M12 4v11"/><path d="m7 10 5 5 5-5"/><path d="M5 20h14"/>"#),
        "cube" => trait_svg(r#"<path d="m12 2.8 8 4.6v9.2l-8 4.6-8-4.6V7.4z"/><path d="m4 7.4 8 4.6 8-4.6"/><path d="M12 12v9.2"/>"#),
        "ecran" => trait_svg(r#"<rect x="2.5" y="4" width="19" height="13" rx="2"/><path d="M8 21h8"/><path d="M12 17v4"/>"#),
        "eclair" => trait_svg(r#"<path d="M13 2 4 14h7l-1 8 9-12h-7z"/>"#),
        "mobile" => trait_svg(r#"<rect x="6.5" y="2.5" width="11" height="19" rx="2.5"/><path d="M11 18.5h2"/>"#),
        "bouclier" => trait_svg(r#"<path d="M12 3 4.5 6v5.5c0 4.6 3.2 8.4 7.5 9.5 4.3-1.1 7.5-4.9 7.5-9.5V6z"/><path d="m9 12 2 2 4-4"/>"#),
        "pinceau" => trait_svg(r#"<path d="M14.5 4.5 19.5 9.5"/><path d="M18 3l3 3-9.5 9.5-3-3z"/><path d="M8.5 12.5C6 12.5 4 14.5 4 17c0 1.5-.5 2.5-1.5 3 4 .5 8-1 8-5z"/>"#),
        "coeur" => trait_svg(r#"<path d="M12 20s-7.5-4.6-7.5-10.2A4.3 4.3 0 0 1 12 7a4.3 4.3 0 0 1 7.5 2.8C19.5 15.4 12 20 12 20z"/>"#),
        "etoile" => trait_svg(r#"<path d="m12 3 2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.4 2.9 1-6.1-4.4-4.3 6.1-.9z"/>"#),
        "check" => trait_svg(r#"<circle class="c" cx="12" cy="12" r="10"/><path class="k" d="m7.5 12.5 3 3 6-6.5"/>"#),
        "croix" => trait_svg(r#"<circle class="c" cx="12" cy="12" r="10"/><path class="k" d="m8.5 8.5 7 7m0-7-7 7"/>"#),
        "camera" => trait_svg(r#"<rect x="2.5" y="6" width="14" height="12" rx="2.5"/><path d="m16.5 10 5-3v10l-5-3"/>"#),
        "liste" => trait_svg(r#"<path d="M9 6h11M9 12h11M9 18h11"/><path d="m3.5 6 1 1 2-2M3.5 12l1 1 2-2M3.5 18l1 1 2-2"/>"#),
        "cible" => trait_svg(r#"<circle cx="12" cy="12" r="9"/><circle cx="12" cy="12" r="5"/><circle cx="12" cy="12" r="1.2" fill="currentColor"/>"#),
        "profil" => trait_svg(r#"<circle cx="9" cy="8" r="3.5"/><path d="M2.5 20a6.5 6.5 0 0 1 13 0"/><path d="M16 4.5a3.5 3.5 0 0 1 0 7M18.5 14.5a6.5 6.5 0 0 1 3 5.5"/>"#),
        "courbe" => trait_svg(r#"<path d="M3 3v18h18"/><path d="m7 15 4-5 3 3 5-7"/>"#),
        "diplome" => trait_svg(r#"<path d="m12 4 10 5-10 5L2 9z"/><path d="M6 11v5c3 2.5 9 2.5 12 0v-5"/><path d="M22 9v6"/>"#),
        "cartes" => trait_svg(r#"<rect x="3" y="5" width="11" height="15" rx="2" transform="rotate(-10 8.5 12.5)"/><rect x="10" y="4" width="11" height="15" rx="2" transform="rotate(8 15.5 11.5)"/>"#),
        "puce" => trait_svg(r#"<rect x="6" y="6" width="12" height="12" rx="2"/><rect x="9.5" y="9.5" width="5" height="5"/><path d="M9 2v4M15 2v4M9 18v4M15 18v4M2 9h4M2 15h4M18 9h4M18 15h4"/>"#),
        "rouage" => trait_svg(r#"<circle cx="12" cy="12" r="3"/><path d="M12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9l2.1 2.1M17 17l2.1 2.1M4.9 19.1 7 17M17 7l2.1-2.1"/>"#),
        "cadenas" => trait_svg(r#"<rect x="4.5" y="10.5" width="15" height="10" rx="2.5"/><path d="M8 10.5V7.5a4 4 0 0 1 8 0v3"/><path d="M12 14.5v2.5"/>"#),
        "linkedin" => plein_svg(r#"<path d="M4.98 3.5a2.5 2.5 0 1 1 0 5 2.5 2.5 0 0 1 0-5zM3 9.75h4V21H3zM9.5 9.75h3.8v1.6h.06c.53-1 1.83-2.05 3.77-2.05 4.03 0 4.77 2.65 4.77 6.1V21h-4v-4.9c0-1.17-.02-2.68-1.63-2.68-1.64 0-1.89 1.28-1.89 2.6V21h-4z"/>"#),
        "github" => plein_svg(r#"<path d="M12 2a10 10 0 0 0-3.16 19.49c.5.09.68-.22.68-.48v-1.7c-2.78.6-3.37-1.34-3.37-1.34-.45-1.16-1.11-1.47-1.11-1.47-.9-.62.07-.6.07-.6 1 .07 1.53 1.03 1.53 1.03.9 1.52 2.34 1.08 2.91.83.09-.65.35-1.09.63-1.34-2.22-.25-4.55-1.11-4.55-4.94 0-1.09.39-1.98 1.03-2.68-.1-.25-.45-1.27.1-2.65 0 0 .84-.27 2.75 1.02a9.6 9.6 0 0 1 5 0c1.91-1.3 2.75-1.02 2.75-1.02.55 1.38.2 2.4.1 2.65.64.7 1.03 1.59 1.03 2.68 0 3.84-2.34 4.68-4.57 4.93.36.31.68.92.68 1.85v2.74c0 .27.18.58.69.48A10 10 0 0 0 12 2z"/>"#),
        "dribbble" => trait_svg(r#"<circle cx="12" cy="12" r="9"/><path d="M8.5 3.7c3.4 4.3 5.6 9.6 6.6 16.3"/><path d="M3.1 10.5c5.3.5 10.8-.6 14.9-4"/><path d="M5.5 18.4c3.7-4.4 9-6 15.3-4.6"/>"#),
        "instagram" => trait_svg(r#"<rect x="3" y="3" width="18" height="18" rx="5"/><circle cx="12" cy="12" r="4"/><circle cx="17.5" cy="6.5" r=".6" fill="currentColor"/>"#),
        _ => String::new(),
    }
}
