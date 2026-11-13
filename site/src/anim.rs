//! Une boucle d'animation (`requestAnimationFrame`), dans le navigateur : la fonction reçoit
//! le temps en millisecondes à chaque image, jusqu'à ce que la boucle soit lâchée.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

type Rappel = Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>>;

/// Une boucle qui tourne : la lâcher l'arrête.
pub struct Boucle {
    actif: Rc<Cell<bool>>,
    image: Rc<Cell<i32>>,
    rappel: Rappel,
}

impl Drop for Boucle {
    fn drop(&mut self) {
        self.actif.set(false);
        if let Some(fenetre) = web_sys::window() {
            let _ = fenetre.cancel_animation_frame(self.image.get());
        }
        // Casse le cycle rappel → cellule → rappel.
        self.rappel.borrow_mut().take();
    }
}

fn demander(rappel: &Closure<dyn FnMut(f64)>) -> i32 {
    web_sys::window()
        .and_then(|w| {
            w.request_animation_frame(rappel.as_ref().unchecked_ref())
                .ok()
        })
        .unwrap_or(0)
}

/// Lance `f` à chaque image.
pub fn boucle(mut f: impl FnMut(f64) + 'static) -> Boucle {
    let actif = Rc::new(Cell::new(true));
    let image = Rc::new(Cell::new(0));
    let rappel: Rappel = Rc::default();
    {
        let (actif, image, soi) = (actif.clone(), image.clone(), rappel.clone());
        *rappel.borrow_mut() = Some(Closure::new(move |temps: f64| {
            if !actif.get() {
                return;
            }
            f(temps);
            if actif.get()
                && let Some(r) = soi.borrow().as_ref()
            {
                image.set(demander(r));
            }
        }));
    }
    if let Some(r) = rappel.borrow().as_ref() {
        image.set(demander(r));
    }
    Boucle {
        actif,
        image,
        rappel,
    }
}
