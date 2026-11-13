//! L'apprentissage des classifieurs (`stratyx_strategy::model::Classifier`) : entropie
//! croisée, mini-lots, optimiseur Adam, régularisation L2, arrêt sur le meilleur score de
//! validation. Rust pur, sur CPU, reproductible à graine égale (décision T8).

use rand::SeedableRng;
use rand::seq::SliceRandom;
use rand_chacha::ChaCha8Rng;
use stratyx_strategy::bundle::Metric;
use stratyx_strategy::model::{Classifier, Dense, Standardizer, softmax};

use crate::dataset::Example;

/// Réglages d'un apprentissage.
#[derive(Debug, Clone)]
pub struct Config {
    pub name: String,
    /// Tailles des couches cachées (vide : régression softmax).
    pub hidden: Vec<usize>,
    pub classes: usize,
    pub epochs: usize,
    pub batch: usize,
    pub learning_rate: f32,
    pub l2: f32,
    pub seed: u64,
}

/// Initialisation de He pour une couche suivie d'une ReLU.
fn init_layer(inputs: usize, outputs: usize, rng: &mut ChaCha8Rng) -> Dense {
    use rand::Rng;
    let mut l = Dense::zeros(inputs, outputs);
    let scale = (2.0 / inputs.max(1) as f32).sqrt();
    for w in &mut l.weights {
        // Somme de trois uniformes : à peu près gaussien.
        let g: f32 = (0..3).map(|_| rng.random::<f32>() - 0.5).sum::<f32>() * 2.0;
        *w = g * scale;
    }
    l
}

struct Adam {
    m: Vec<Vec<f32>>,
    v: Vec<Vec<f32>>,
    t: i32,
}

impl Adam {
    fn new(model: &Classifier) -> Adam {
        let sizes: Vec<usize> = model
            .layers
            .iter()
            .flat_map(|l| [l.weights.len(), l.bias.len()])
            .collect();
        Adam {
            m: sizes.iter().map(|s| vec![0.0; *s]).collect(),
            v: sizes.iter().map(|s| vec![0.0; *s]).collect(),
            t: 0,
        }
    }

    fn step(&mut self, model: &mut Classifier, grads: &[Vec<f32>], lr: f32) {
        self.t += 1;
        let (b1, b2, eps) = (0.9f32, 0.999f32, 1e-8f32);
        let c1 = 1.0 - b1.powi(self.t);
        let c2 = 1.0 - b2.powi(self.t);
        let mut k = 0;
        for layer in &mut model.layers {
            for params in [&mut layer.weights, &mut layer.bias] {
                let (m, v, g) = (&mut self.m[k], &mut self.v[k], &grads[k]);
                for i in 0..params.len() {
                    m[i] = b1 * m[i] + (1.0 - b1) * g[i];
                    v[i] = b2 * v[i] + (1.0 - b2) * g[i] * g[i];
                    params[i] -= lr * (m[i] / c1) / ((v[i] / c2).sqrt() + eps);
                }
                k += 1;
            }
        }
    }
}

/// Gradients de l'entropie croisée pour un exemple, ajoutés à `grads`. Renvoie la perte.
fn backprop(model: &Classifier, x: &[f32], y: usize, grads: &mut [Vec<f32>]) -> f32 {
    let acts = model.forward_all(x);
    let out = acts.last().expect("sortie");
    let p = softmax(out);
    let loss = -p[y].max(1e-7).ln();
    let mut delta: Vec<f32> = p.clone();
    delta[y] -= 1.0;
    for li in (0..model.layers.len()).rev() {
        let layer = &model.layers[li];
        // Entrée de la couche : activation précédente (après ReLU sauf pour l'entrée).
        let input: Vec<f32> = if li == 0 {
            acts[0].clone()
        } else {
            acts[li].iter().map(|v| v.max(0.0)).collect()
        };
        let (gw, gb) = (2 * li, 2 * li + 1);
        for o in 0..layer.outputs {
            let d = delta[o];
            if d == 0.0 {
                continue;
            }
            grads[gb][o] += d;
            let row = &mut grads[gw][o * layer.inputs..(o + 1) * layer.inputs];
            for (g, xi) in row.iter_mut().zip(&input) {
                *g += d * xi;
            }
        }
        if li > 0 {
            let mut prev = vec![0.0f32; layer.inputs];
            for (o, d) in delta.iter().enumerate() {
                let row = &layer.weights[o * layer.inputs..(o + 1) * layer.inputs];
                for (pv, w) in prev.iter_mut().zip(row) {
                    *pv += d * w;
                }
            }
            // Dérivée de la ReLU.
            for (pv, z) in prev.iter_mut().zip(&acts[li]) {
                if *z <= 0.0 {
                    *pv = 0.0;
                }
            }
            delta = prev;
        }
    }
    loss
}

/// Précision, perte et score de la classe majoritaire sur des exemples.
#[must_use]
pub fn evaluate(model: &Classifier, data: &[Example], classes: usize) -> (f32, f32, f32) {
    if data.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let mut correct = 0usize;
    let mut loss = 0.0f64;
    let mut counts = vec![0usize; classes];
    for e in data {
        let p = model.predict(&e.x);
        let best = p
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map_or(0, |(i, _)| i);
        correct += usize::from(best == e.y);
        loss += f64::from(-p[e.y].max(1e-7).ln());
        counts[e.y.min(classes - 1)] += 1;
    }
    let n = data.len() as f32;
    (
        correct as f32 / n,
        (loss / data.len() as f64) as f32,
        *counts.iter().max().unwrap_or(&0) as f32 / n,
    )
}

/// Entraîne un classifieur ; garde les poids du meilleur score de validation.
pub fn train(
    config: &Config,
    train: &[Example],
    validation: &[Example],
    mut log: impl FnMut(&str),
) -> (Classifier, Metric) {
    let mut rng = ChaCha8Rng::seed_from_u64(config.seed);
    let rows: Vec<Vec<f32>> = train.iter().map(|e| e.x.clone()).collect();
    let dim = rows.first().map_or(0, Vec::len);
    let standardizer = Standardizer::fit(&rows);
    let mut sizes = vec![dim];
    sizes.extend(&config.hidden);
    sizes.push(config.classes);
    let layers = sizes
        .windows(2)
        .map(|w| init_layer(w[0], w[1], &mut rng))
        .collect();
    let mut model = Classifier {
        name: config.name.clone(),
        standardizer,
        layers,
    };
    let mut adam = Adam::new(&model);
    let mut best = (f32::INFINITY, model.clone());
    let mut idx: Vec<usize> = (0..train.len()).collect();
    for epoch in 0..config.epochs {
        idx.shuffle(&mut rng);
        let lr = config.learning_rate
            * 0.5f32.powf(epoch as f32 / (config.epochs as f32 / 2.0).max(1.0));
        let mut total = 0.0f64;
        for chunk in idx.chunks(config.batch.max(1)) {
            let mut grads: Vec<Vec<f32>> = model
                .layers
                .iter()
                .flat_map(|l| [vec![0.0; l.weights.len()], vec![0.0; l.bias.len()]])
                .collect();
            for &i in chunk {
                total += f64::from(backprop(&model, &train[i].x, train[i].y, &mut grads));
            }
            let scale = 1.0 / chunk.len() as f32;
            for (k, g) in grads.iter_mut().enumerate() {
                let is_weight = k % 2 == 0;
                let layer = &model.layers[k / 2];
                for (j, v) in g.iter_mut().enumerate() {
                    *v *= scale;
                    if is_weight {
                        *v += config.l2 * layer.weights[j];
                    }
                }
            }
            adam.step(&mut model, &grads, lr);
        }
        let (acc, val_loss, _) = evaluate(&model, validation, config.classes);
        log(&format!(
            "    {} · époque {:>2} · perte {:.4} · validation {:.4} · précision {:.1} %",
            config.name,
            epoch + 1,
            total / train.len().max(1) as f64,
            val_loss,
            acc * 100.0
        ));
        if val_loss < best.0 {
            best = (val_loss, model.clone());
        }
    }
    let model = best.1;
    let (accuracy, log_loss, baseline) = evaluate(&model, validation, config.classes);
    let metric = Metric {
        model: config.name.clone(),
        train: train.len(),
        validation: validation.len(),
        accuracy,
        log_loss,
        baseline,
        parameters: model.parameters(),
    };
    (model, metric)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learns_a_separable_problem() {
        // y = 1 si x0 > x1, sinon 0 ; plus une classe 2 quand x0 + x1 > 1,5.
        let mut data = Vec::new();
        let mut rng = ChaCha8Rng::seed_from_u64(1);
        use rand::Rng;
        for i in 0..3000u64 {
            let (a, b): (f32, f32) = (rng.random(), rng.random());
            let y = if a + b > 1.5 { 2 } else { usize::from(a > b) };
            data.push(Example {
                x: vec![a, b],
                y,
                hand: i,
            });
        }
        let (tr, va) = crate::dataset::split(data);
        let config = Config {
            name: "t".into(),
            hidden: vec![16],
            classes: 3,
            epochs: 15,
            batch: 32,
            learning_rate: 0.01,
            l2: 1e-5,
            seed: 3,
        };
        let (_, m) = train(&config, &tr, &va, |_| {});
        assert!(m.accuracy > 0.93, "{m:?}");
        let linear = Config {
            hidden: vec![],
            ..config
        };
        let (_, m) = train(&linear, &tr, &va, |_| {});
        assert!(m.accuracy > 0.85, "{m:?}");
    }
}
