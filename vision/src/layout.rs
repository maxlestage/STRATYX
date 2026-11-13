//! Où sont les cartes : le board (une rangée de 3 à 5 cartes semblables vers le milieu),
//! les cartes du joueur (deux cartes côte à côte ou qui se chevauchent, en bas et au
//! centre), et les autres (cartes montrées par les adversaires).

use crate::detect::DetectedCard;
use crate::frame::Frame;

/// La lecture d'une table : index des cartes détectées.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TableRead {
    /// Les cartes du joueur (au plus deux, de gauche à droite).
    pub hero: Vec<usize>,
    /// Le board, de gauche à droite (au plus cinq).
    pub board: Vec<usize>,
    /// Les autres cartes visibles.
    pub others: Vec<usize>,
}

fn similar(a: f32, b: f32, tolerance: f32) -> bool {
    let r = a / b.max(1.0);
    r > 1.0 - tolerance && r < 1.0 / (1.0 - tolerance)
}

/// Range les cartes détectées en cartes du joueur, board et autres.
#[must_use]
pub fn arrange(frame: &Frame, cards: &[DetectedCard]) -> TableRead {
    let fw = frame.width as f32;
    let fh = frame.height as f32;
    let centers: Vec<(f32, f32)> = cards.iter().map(|c| c.rect.center()).collect();
    let heights: Vec<f32> = cards.iter().map(|c| c.rect.h as f32).collect();

    // Le board : la meilleure rangée de 3 à 5 cartes.
    let mut board: Vec<usize> = Vec::new();
    let mut best_score = f32::MIN;
    for seed in 0..cards.len() {
        let mut row: Vec<usize> = (0..cards.len())
            .filter(|&j| {
                similar(heights[j], heights[seed], 0.15)
                    && (centers[j].1 - centers[seed].1).abs() < 0.3 * heights[seed]
            })
            .collect();
        row.sort_by(|a, b| centers[*a].0.total_cmp(&centers[*b].0));
        // Coupe la rangée aux grands écarts : les cartes du board se suivent.
        let mut runs: Vec<Vec<usize>> = vec![Vec::new()];
        for &j in &row {
            if let Some(&prev) = runs.last().and_then(|r| r.last()) {
                let gap = cards[j].rect.x as f32 - (cards[prev].rect.x + cards[prev].rect.w) as f32;
                if gap > 0.9 * cards[prev].rect.w as f32 {
                    runs.push(Vec::new());
                }
                // Des cartes qui se chevauchent beaucoup sont une main, pas un board.
                if gap < -0.25 * cards[prev].rect.w as f32 {
                    runs.push(Vec::new());
                }
            }
            if let Some(r) = runs.last_mut() {
                r.push(j);
            }
        }
        for mut run in runs {
            if run.len() < 3 {
                continue;
            }
            while run.len() > 5 {
                // Garde les cinq plus proches du centre horizontal.
                let far = run
                    .iter()
                    .enumerate()
                    .max_by(|a, b| {
                        (centers[*a.1].0 - fw / 2.0)
                            .abs()
                            .total_cmp(&(centers[*b.1].0 - fw / 2.0).abs())
                    })
                    .map_or(0, |(i, _)| i);
                run.remove(far);
            }
            let cy = centers[run[0]].1 / fh;
            let cx = run.iter().map(|j| centers[*j].0).sum::<f32>() / run.len() as f32 / fw;
            let score = run.len() as f32 * 2.0 - (cy - 0.45).abs() * 4.0 - (cx - 0.5).abs() * 2.0;
            if score > best_score {
                best_score = score;
                board = run;
            }
        }
    }

    // Les cartes du joueur : la meilleure paire restante, en bas et au centre.
    let rest: Vec<usize> = (0..cards.len()).filter(|j| !board.contains(j)).collect();
    let mut hero: Vec<usize> = Vec::new();
    let mut best_score = f32::MIN;
    for (k, &a) in rest.iter().enumerate() {
        for &b in &rest[k + 1..] {
            let wmax = cards[a].rect.w.max(cards[b].rect.w) as f32;
            if !similar(heights[a], heights[b], 0.25)
                || (centers[a].0 - centers[b].0).abs() > 1.35 * wmax
                || (centers[a].1 - centers[b].1).abs() > 0.5 * heights[a].max(heights[b])
            {
                continue;
            }
            let cy = (centers[a].1 + centers[b].1) / 2.0 / fh;
            let cx = (centers[a].0 + centers[b].0) / 2.0 / fw;
            let score = cy - (cx - 0.5).abs() * 0.6;
            if score > best_score {
                best_score = score;
                hero = vec![a, b];
            }
        }
    }
    if hero.is_empty() {
        // Une seule carte en bas au centre : sans doute une carte du joueur à moitié cachée.
        if let Some(&a) = rest
            .iter()
            .find(|&&a| centers[a].1 / fh > 0.6 && (centers[a].0 / fw - 0.5).abs() < 0.2)
        {
            hero.push(a);
        }
    }
    hero.sort_by(|a, b| centers[*a].0.total_cmp(&centers[*b].0));
    let others = rest.into_iter().filter(|j| !hero.contains(j)).collect();
    TableRead {
        hero,
        board,
        others,
    }
}
