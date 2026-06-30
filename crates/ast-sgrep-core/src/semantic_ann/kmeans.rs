use ast_sgrep_embed::cosine_similarity;

use super::vector::normalize_vec;

pub(crate) fn kmeans(
    vectors: &[Vec<f32>],
    k: usize,
    max_iters: usize,
) -> (Vec<Vec<f32>>, Vec<usize>) {
    let n = vectors.len();
    let k = k.min(n).max(1);
    let dim = vectors[0].len();

    let mut centroids: Vec<Vec<f32>> = Vec::with_capacity(k);
    centroids.push(vectors[0].clone());
    while centroids.len() < k {
        let mut best_idx = 0usize;
        let mut best_dist = -1.0f32;
        for (i, v) in vectors.iter().enumerate() {
            let min_sim = centroids
                .iter()
                .map(|c| cosine_similarity(v, c))
                .fold(f32::INFINITY, f32::min);
            let dist = 1.0 - min_sim;
            if dist > best_dist {
                best_dist = dist;
                best_idx = i;
            }
        }
        centroids.push(vectors[best_idx].clone());
    }

    let mut assignments = vec![0usize; n];
    for _ in 0..max_iters {
        let mut changed = false;
        for (i, v) in vectors.iter().enumerate() {
            let (best, _) = centroids
                .iter()
                .enumerate()
                .map(|(ci, c)| (ci, cosine_similarity(v, c)))
                .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                .unwrap_or((0, 0.0));
            if assignments[i] != best {
                assignments[i] = best;
                changed = true;
            }
        }
        if !changed {
            break;
        }
        let mut sums = vec![vec![0.0f32; dim]; k];
        let mut counts = vec![0usize; k];
        for (i, v) in vectors.iter().enumerate() {
            let c = assignments[i];
            counts[c] += 1;
            for (j, val) in v.iter().enumerate() {
                sums[c][j] += val;
            }
        }
        for ci in 0..k {
            if counts[ci] > 0 {
                centroids[ci] = normalize_vec(
                    &sums[ci]
                        .iter()
                        .map(|s| s / counts[ci] as f32)
                        .collect::<Vec<_>>(),
                );
            }
        }
    }
    (centroids, assignments)
}
