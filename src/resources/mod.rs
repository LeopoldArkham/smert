pub mod artwork;
pub mod definition;
pub mod quote;
pub mod scripture;

use rand::seq::IndexedRandom;

pub trait DailyResource: Sized + Clone {

    /// Pick `n` distinct items at random (uniform, without replacement).
    /// Returns fewer than `n` if the collection is smaller.
    fn pick_n(items: &[Self], n: usize) -> Vec<Self> {
        let mut rng = rand::rng();
        items.sample(&mut rng, n).cloned().collect()
    }
}
