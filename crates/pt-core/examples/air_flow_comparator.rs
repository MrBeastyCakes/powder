//! Bounded CPU research comparator, not the production air backend.
//!
//! Cordier, Degond & Kumbaro (2012), arXiv:1108.2876, equations
//! (2.12)--(2.15), (3.1)--(3.2), (3.9)--(3.10), with a constant-gamma EOS,
//! alpha=1, periodic uniform 1D cells, centered implicit operators and the
//! published Lax--Friedrichs explicit dissipation. Crucially, the thermodynamic
//! closure uses OLD kinetic energy as printed in (2.15). It is not silently
//! replaced by the new physical kinetic energy. Both energies are measured.
//!
//! This derives the pressure matrix by substituting the momentum update into
//! the conservative energy flux (2.14) with the old kinetic term from (2.15).
//! No variable-cp mixture, viscosity, gravity, wall or vacuum claim is made.
//! The dense pivoted solve is deliberately small and unsuitable for production.
//!
//! cargo run -p pt-core --example air_flow_comparator

const GAMMA: f64 = 1.4;

#[derive(Clone, Copy)]
struct Cell {
    rho: f64,
    momentum: f64,
    energy: f64,
    pressure: f64,
}

impl Cell {
    fn initial(rho: f64, velocity: f64, pressure: f64, epsilon: f64) -> Self {
        Self {
            rho,
            momentum: rho * velocity,
            energy: pressure / (GAMMA - 1.0) + 0.5 * epsilon.powi(2) * rho * velocity.powi(2),
            pressure,
        }
    }

    fn kinetic(self, epsilon: f64) -> f64 {
        0.5 * epsilon.powi(2) * self.momentum.powi(2) / self.rho
    }
}

struct Comparator {
    cells: Vec<Cell>,
    epsilon: f64,
    dx: f64,
    time: f64,
    max_pressure_residual: f64,
    max_eos_defect: f64,
}

impl Comparator {
    fn new(n: usize, epsilon: f64, initial: impl Fn(f64) -> (f64, f64, f64)) -> Self {
        assert!((8..=256).contains(&n) && epsilon > 0.0 && epsilon <= 1.0);
        let dx = 1.0 / n as f64;
        let cells = (0..n)
            .map(|i| {
                let (rho, u, p) = initial((i as f64 + 0.5) * dx);
                Cell::initial(rho, u, p, epsilon)
            })
            .collect();
        Self {
            cells,
            epsilon,
            dx,
            time: 0.0,
            max_pressure_residual: 0.0,
            max_eos_defect: 0.0,
        }
    }

    fn inventories(&self) -> [f64; 4] {
        self.cells.iter().fold([0.0; 4], |mut sum, c| {
            sum[0] += self.dx * c.rho;
            sum[1] += self.dx * c.momentum;
            sum[2] += self.dx * c.energy;
            sum[3] += self.dx * (c.pressure / (GAMMA - 1.0) + c.kinetic(self.epsilon));
            sum
        })
    }

    /// Tentative computation lives in local vectors. Failed solves do not
    /// change the accepted state or advance time. No source/retry machinery is
    /// claimed by this source-free research comparator.
    fn step(&mut self, dt: f64) -> Result<(), &'static str> {
        if !dt.is_finite() || dt <= 0.0 {
            return Err("invalid timestep");
        }
        let n = self.cells.len();
        let left = |i: usize| (i + n - 1) % n;
        let right = |i: usize| (i + 1) % n;
        let mut face = vec![[0.0; 3]; n];
        for (i, flux) in face.iter_mut().enumerate() {
            let a = self.cells[i];
            let b = self.cells[right(i)];
            let ua = a.momentum / a.rho;
            let ub = b.momentum / b.rho;
            // (3.10), alpha=1; am=sqrt(gamma*p/rho) in these scaled units.
            // The actual scaled acoustic speed is am/epsilon.
            let speed = (ua.abs() + (GAMMA * a.pressure / a.rho).sqrt()).max(ub.abs() + (GAMMA * b.pressure / b.rho).sqrt());
            if speed * dt / self.dx > 0.45 {
                return Err("explicit flux CFL exceeds research limit");
            }
            *flux = [
                0.5 * (a.momentum + b.momentum) - 0.5 * speed * (b.rho - a.rho),
                0.5 * (a.momentum * ua + a.pressure + b.momentum * ub + b.pressure) - 0.5 * speed * (b.momentum - a.momentum),
                // Only the explicit energy dissipation. H*q is implicit below.
                -0.5 * speed * (b.energy - a.energy),
            ];
        }
        let mut rho = vec![0.0; n];
        let mut q_star = vec![0.0; n];
        let mut u_star = vec![0.0; n];
        let mut enthalpy = vec![0.0; n];
        for i in 0..n {
            let c = self.cells[i];
            let divergence = |field| (face[i][field] - face[left(i)][field]) / self.dx;
            rho[i] = c.rho - dt * divergence(0);
            q_star[i] = c.momentum - dt * divergence(1);
            u_star[i] = c.energy - dt * divergence(2);
            // H is TOTAL specific enthalpy in Cordier, unlike the static
            // enthalpy symbol used by Tavelli--Dumbser.
            enthalpy[i] = (c.energy + c.pressure) / c.rho;
            if !rho[i].is_finite() || rho[i] <= 0.0 {
                return Err("nonpositive tentative density");
            }
        }
        // D and G are both the central first derivative. Assemble the actual
        // composed operator D(H G), which has a five-point +/-2 stencil.
        // Substitution of (2.13) into (2.14), closure (2.15):
        // [I/(gamma-1)-dt^2*A*D(H G)] p
        //   = U* - K_old - dt*D(H q*), A=(1-epsilon^2)/epsilon^2.
        let coefficient = (1.0 - self.epsilon.powi(2)) / self.epsilon.powi(2);
        let scale = dt.powi(2) * coefficient / (4.0 * self.dx.powi(2));
        let mut matrix = vec![vec![0.0; n]; n];
        let mut rhs = vec![0.0; n];
        for i in 0..n {
            let l = left(i);
            let r = right(i);
            matrix[i][i] = 1.0 / (GAMMA - 1.0) + scale * (enthalpy[l] + enthalpy[r]);
            matrix[i][left(l)] -= scale * enthalpy[l];
            matrix[i][right(r)] -= scale * enthalpy[r];
            rhs[i] = u_star[i]
                - self.cells[i].kinetic(self.epsilon)
                - dt * (enthalpy[r] * q_star[r] - enthalpy[l] * q_star[l]) / (2.0 * self.dx);
        }
        let pressure = solve(matrix.clone(), rhs.clone())?;
        let mut residual: f64 = 0.0;
        for i in 0..n {
            let ap: f64 = matrix[i].iter().zip(&pressure).map(|(a, p)| a * p).sum();
            residual = residual.max((ap - rhs[i]).abs() / rhs[i].abs().max(1.0));
        }
        if !residual.is_finite() || residual > 1e-8 {
            return Err("pressure solve did not converge");
        }
        let mut next = Vec::with_capacity(n);
        let mut eos_defect: f64 = 0.0;
        for i in 0..n {
            let momentum = q_star[i] - dt * coefficient * (pressure[right(i)] - pressure[left(i)]) / (2.0 * self.dx);
            let c = Cell {
                rho: rho[i],
                momentum,
                energy: pressure[i] / (GAMMA - 1.0) + self.cells[i].kinetic(self.epsilon),
                pressure: pressure[i],
            };
            let physical_pressure = (GAMMA - 1.0) * (c.energy - c.kinetic(self.epsilon));
            if !c.energy.is_finite()
                || !c.momentum.is_finite()
                || !physical_pressure.is_finite()
                || c.pressure <= 0.0
                || physical_pressure <= 0.0
            {
                return Err("invalid tentative thermodynamic state");
            }
            eos_defect = eos_defect.max((physical_pressure - c.pressure).abs());
            next.push(c);
        }
        self.cells = next;
        self.time += dt;
        self.max_pressure_residual = self.max_pressure_residual.max(residual);
        self.max_eos_defect = self.max_eos_defect.max(eos_defect);
        Ok(())
    }
}

fn solve(mut a: Vec<Vec<f64>>, mut b: Vec<f64>) -> Result<Vec<f64>, &'static str> {
    let n = b.len();
    for k in 0..n {
        let pivot = (k..n)
            .max_by(|&i, &j| a[i][k].abs().total_cmp(&a[j][k].abs()))
            .ok_or("empty pressure system")?;
        if !a[pivot][k].is_finite() || a[pivot][k].abs() < 1e-14 {
            return Err("singular pressure system");
        }
        a.swap(k, pivot);
        b.swap(k, pivot);
        let pivot_b = b[k];
        let (through_pivot, remaining) = a.split_at_mut(k + 1);
        let pivot_row = &through_pivot[k];
        for (row, rhs) in remaining.iter_mut().zip(&mut b[k + 1..]) {
            let factor = row[k] / pivot_row[k];
            row[k] = 0.0;
            for (entry, pivot_entry) in row[k + 1..].iter_mut().zip(&pivot_row[k + 1..]) {
                *entry -= factor * pivot_entry;
            }
            *rhs -= factor * pivot_b;
        }
    }
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        let tail: f64 = (i + 1..n).map(|j| a[i][j] * x[j]).sum();
        x[i] = (b[i] - tail) / a[i][i];
    }
    Ok(x)
}

fn run(mut sim: Comparator, dt: f64, steps: usize) -> Result<Comparator, &'static str> {
    for _ in 0..steps {
        sim.step(dt)?;
    }
    Ok(sim)
}

fn acoustic(n: usize, epsilon: f64) -> Result<(Comparator, f64, [f64; 4]), &'static str> {
    acoustic_steps(n, epsilon, 0.02 * epsilon / n as f64, 5 * n)
}

fn acoustic_steps(n: usize, epsilon: f64, dt: f64, steps: usize) -> Result<(Comparator, f64, [f64; 4]), &'static str> {
    let amplitude = 1e-4;
    let speed = GAMMA.sqrt() / epsilon;
    let sim = Comparator::new(n, epsilon, |x| {
        let wave = amplitude * (std::f64::consts::TAU * x).sin();
        (1.0 + wave, speed * wave, 1.0 + GAMMA * wave)
    });
    let before = sim.inventories();
    let sim = run(sim, dt, steps)?;
    let error: f64 = sim
        .cells
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let x = (i as f64 + 0.5) * sim.dx;
            let exact = 1.0 + GAMMA * amplitude * (std::f64::consts::TAU * (x - speed * sim.time)).sin();
            (c.pressure - exact).abs() * sim.dx
        })
        .sum();
    Ok((sim, error / (GAMMA * amplitude), before))
}

fn main() -> Result<(), &'static str> {
    println!("Cordier 2012 first-order periodic 1D comparator; gamma=1.4, alpha=1.");
    println!("Frozen kinetic-energy closure is retained and its physical EOS defect is reported.");
    println!("case,cells,epsilon,time,relative_mass_drift,relative_stored_energy_drift,relative_physical_energy_drift,max_pressure_residual,max_pressure_EOS_defect,pressure_L1_over_wave_amplitude");
    for epsilon in [1.0, 0.1] {
        for n in [32, 64] {
            let (sim, error, before) = acoustic(n, epsilon)?;
            print_result("acoustic", &sim, before, error);
        }
    }
    // A large acoustic step remains stable but cannot resolve a wave that
    // traverses more than a domain length during that single step.
    let (sim, error, before) = acoustic_steps(64, 1e-3, 0.001, 1)?;
    print_result("underresolved_acoustic", &sim, before, error);
    let sim = Comparator::new(64, 1e-3, |x| (1.0 + 0.1 * (std::f64::consts::TAU * x).sin(), 0.1, 1.0));
    let before = sim.inventories();
    let sim = run(sim, 0.001, 100)?;
    let pressure_error = sim.cells.iter().map(|c| (c.pressure - 1.0).abs()).fold(0.0, f64::max);
    print_result("low_mach_contact", &sim, before, pressure_error);
    println!("Scope: source-free calorically perfect gas only; no production solver gate is closed.");
    Ok(())
}

fn print_result(case: &str, sim: &Comparator, before: [f64; 4], error: f64) {
    let after = sim.inventories();
    let relative = |i: usize| (after[i] - before[i]) / before[i].abs().max(1e-30);
    println!(
        "{case},{},{:.3e},{:.6},{:.6e},{:.6e},{:.6e},{:.6e},{:.6e},{error:.6e}",
        sim.cells.len(),
        sim.epsilon,
        sim.time,
        relative(0),
        relative(2),
        relative(3),
        sim.max_pressure_residual,
        sim.max_eos_defect
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_moving_gas_is_preserved_at_low_mach() {
        let sim = Comparator::new(16, 0.001, |_| (1.0, 0.1, 1.0));
        let sim = run(sim, 0.001, 10).unwrap();
        for c in sim.cells {
            assert!((c.rho - 1.0).abs() < 1e-10);
            assert!((c.momentum - 0.1).abs() < 1e-10);
            assert!((c.pressure - 1.0).abs() < 1e-10);
        }
    }

    #[test]
    fn rejected_step_preserves_state_and_time() {
        let mut sim = Comparator::new(16, 0.01, |_| (1.0, 0.1, 1.0));
        let before = sim.inventories();
        assert!(sim.step(1.0).is_err());
        assert_eq!(sim.time, 0.0);
        assert_eq!(sim.inventories(), before);
    }

    #[test]
    fn small_acoustic_wave_converges_and_conserves_stored_energy() {
        let (_, coarse_error, _) = acoustic(32, 0.1).unwrap();
        let (fine, fine_error, before) = acoustic(64, 0.1).unwrap();
        assert!(fine_error < coarse_error * 0.8);
        assert!(fine_error < 0.08);
        let after = fine.inventories();
        assert!((after[0] - before[0]).abs() < 1e-10);
        assert!((after[2] - before[2]).abs() < 1e-10);
        assert!(fine.max_eos_defect > 0.0, "must expose the frozen-K defect");
    }

    #[test]
    fn implicit_stability_does_not_establish_wave_accuracy() {
        let (sim, error, before) = acoustic_steps(64, 1e-3, 0.001, 1).unwrap();
        assert!(error > 0.1, "an unresolved acoustic step must be reported as inaccurate");
        let after = sim.inventories();
        assert!((after[0] - before[0]).abs() < 1e-10);
        assert!((after[2] - before[2]).abs() < 1e-10);
        assert!(sim.max_pressure_residual < 1e-8);
    }
}
