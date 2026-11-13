//! Les modèles appris, en Rust pur : régression softmax et perceptron multicouche (ReLU).
//!
//! Ils ne font ici que prédire ; l'entraînement est dans `stratyx-ai-training`. Les poids
//! sont sérialisés en JSON (fichiers `models/*.json` des IA), embarqués dans les binaires
//! et dans le WebAssembly du site.

use serde::{Deserialize, Serialize};

/// Centre et réduit les caractéristiques (moyennes et écarts-types de l'entraînement).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Standardizer {
    pub mean: Vec<f32>,
    pub std: Vec<f32>,
}

impl Standardizer {
    /// Calculé sur des exemples.
    #[must_use]
    pub fn fit(rows: &[Vec<f32>]) -> Standardizer {
        let d = rows.first().map_or(0, Vec::len);
        let n = rows.len().max(1) as f64;
        let mut mean = vec![0.0f64; d];
        for r in rows {
            for (m, x) in mean.iter_mut().zip(r) {
                *m += f64::from(*x);
            }
        }
        for m in &mut mean {
            *m /= n;
        }
        let mut var = vec![0.0f64; d];
        for r in rows {
            for ((v, x), m) in var.iter_mut().zip(r).zip(&mean) {
                *v += (f64::from(*x) - m).powi(2);
            }
        }
        Standardizer {
            mean: mean.iter().map(|m| *m as f32).collect(),
            std: var
                .iter()
                .map(|v| ((v / n).sqrt() as f32).max(1e-4))
                .collect(),
        }
    }

    #[must_use]
    pub fn apply(&self, x: &[f32]) -> Vec<f32> {
        x.iter()
            .zip(self.mean.iter().zip(&self.std))
            .map(|(x, (m, s))| (x - m) / s)
            .collect()
    }
}

/// Une couche dense : `out = W · x + b` (W en lignes de sortie).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Dense {
    pub inputs: usize,
    pub outputs: usize,
    /// `outputs × inputs`, ligne par ligne.
    pub weights: Vec<f32>,
    pub bias: Vec<f32>,
}

impl Dense {
    #[must_use]
    pub fn zeros(inputs: usize, outputs: usize) -> Dense {
        Dense {
            inputs,
            outputs,
            weights: vec![0.0; inputs * outputs],
            bias: vec![0.0; outputs],
        }
    }

    #[must_use]
    pub fn forward(&self, x: &[f32]) -> Vec<f32> {
        (0..self.outputs)
            .map(|o| {
                let row = &self.weights[o * self.inputs..(o + 1) * self.inputs];
                self.bias[o] + row.iter().zip(x).map(|(w, x)| w * x).sum::<f32>()
            })
            .collect()
    }
}

/// Probabilités à partir de scores.
#[must_use]
pub fn softmax(z: &[f32]) -> Vec<f32> {
    let max = z.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let e: Vec<f32> = z.iter().map(|v| (v - max).exp()).collect();
    let s: f32 = e.iter().sum();
    e.iter().map(|v| v / s.max(1e-12)).collect()
}

/// Un classifieur : couches denses, ReLU entre elles, softmax à la fin. Sans couche
/// cachée, c'est une régression softmax (modèle linéaire).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Classifier {
    /// Nom court (`decision-mlp`, `population-softmax`…).
    pub name: String,
    pub standardizer: Standardizer,
    pub layers: Vec<Dense>,
}

impl Classifier {
    /// Activations de chaque couche (avant ReLU), pour l'entraînement.
    #[must_use]
    pub fn forward_all(&self, x: &[f32]) -> Vec<Vec<f32>> {
        let mut acts = Vec::with_capacity(self.layers.len() + 1);
        let mut h = self.standardizer.apply(x);
        acts.push(h.clone());
        for (i, layer) in self.layers.iter().enumerate() {
            let mut z = layer.forward(&h);
            acts.push(z.clone());
            if i + 1 < self.layers.len() {
                for v in &mut z {
                    *v = v.max(0.0);
                }
            }
            h = z;
        }
        acts
    }

    /// Probabilités de chaque classe.
    #[must_use]
    pub fn predict(&self, x: &[f32]) -> Vec<f32> {
        let acts = self.forward_all(x);
        softmax(acts.last().map_or(&[][..], Vec::as_slice))
    }

    /// La classe la plus probable et sa probabilité.
    #[must_use]
    pub fn best(&self, x: &[f32]) -> (usize, f32) {
        let p = self.predict(x);
        p.iter()
            .enumerate()
            .fold((0, 0.0), |b, (i, v)| if *v > b.1 { (i, *v) } else { b })
    }

    /// Nombre de paramètres.
    #[must_use]
    pub fn parameters(&self) -> usize {
        self.layers
            .iter()
            .map(|l| l.weights.len() + l.bias.len())
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_and_mlp_forward() {
        let st = Standardizer::fit(&[vec![0.0, 2.0], vec![2.0, 2.0]]);
        assert_eq!(st.mean, vec![1.0, 2.0]);
        let mut l = Dense::zeros(2, 3);
        l.bias = vec![0.0, 1.0, 0.0];
        let c = Classifier {
            name: "t".into(),
            standardizer: st,
            layers: vec![l],
        };
        assert_eq!(c.best(&[1.0, 2.0]).0, 1);
        let p = c.predict(&[0.0, 0.0]);
        assert!((p.iter().sum::<f32>() - 1.0).abs() < 1e-6);
        let json = serde_json::to_string(&c).unwrap();
        let back: Classifier = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
    }
}
