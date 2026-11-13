//! Recalcule `src/tables.rs` : `cargo run --release -p stratyx-strategy --example tables > strategy/src/tables.rs`.

use stratyx_strategy::preflop::{CLASSES, class_at, compute_tables};

fn main() {
    let samples: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(200_000);
    let t = compute_tables(samples, 0x5354_5241_5459_5821);
    println!(
        "//! Force des 169 classes de mains de départ : équité préflop contre 1, 2 et 5 mains"
    );
    println!("//! au hasard, dans l'ordre de la grille (voir [`crate::preflop`]).");
    println!("//!");
    println!("//! Fichier généré par `cargo run --release -p stratyx-strategy --example tables`");
    println!("//! ({samples} tirages par classe, graine fixe) : ne pas modifier à la main.");
    println!();
    for (name, values) in ["VS1", "VS2", "VS5"].iter().zip(t.iter()) {
        println!("#[rustfmt::skip]");
        println!("pub const {name}: [f32; {CLASSES}] = [");
        for row in 0..13 {
            let cells: Vec<String> = (0..13)
                .map(|c| format!("{:.4}", values[row * 13 + c]))
                .collect();
            let labels: Vec<String> = (0..13)
                .map(|c| class_at(row * 13 + c).to_string())
                .collect();
            println!("    {}, // {}", cells.join(", "), labels.join(" "));
        }
        println!("];");
        println!();
    }
}
