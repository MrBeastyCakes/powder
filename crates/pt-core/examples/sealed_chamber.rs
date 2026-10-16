//! Run with: cargo run -p pt-core --example sealed_chamber
//! Uniform thermodynamics only; no airflow or GPU performance is measured.
use pt_core::gas::{SealedChamber, ThermodynamicModel, WorldMetadata};
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let metadata = WorldMetadata::sealed_reference(1000, 1000, ThermodynamicModel::ConstantCvReference);
    let mut chamber = SealedChamber::new(metadata)?;
    let before = chamber.properties()?;
    let power_w = 1000.0;
    let duration = Duration::from_secs(10);
    let expected_t = before.temperature_k + power_w * duration.as_secs_f64() / (before.mass_kg * before.cv_j_per_kg_k);
    chamber.advance_uniform_heater(power_w, duration)?;
    let after = chamber.properties()?;
    let restored = SealedChamber::from_toml(&chamber.to_toml()?)?;
    assert_eq!(restored, chamber);
    println!("Uniform sealed gas reference: rigid adiabatic 1 m³, gravity off");
    println!(
        "Injected heat: {:.6} J over {:.3} s",
        chamber.ledger().external_heat_j,
        chamber.elapsed().as_secs_f64()
    );
    println!("Mass: {:.12} kg -> {:.12} kg", before.mass_kg, after.mass_kg);
    println!(
        "Temperature: {:.6} K -> {:.6} K (analytic {:.6} K)",
        before.temperature_k, after.temperature_k, expected_t
    );
    println!(
        "Absolute pressure: {:.6} Pa -> {:.6} Pa (analytic {:.6} Pa)",
        before.absolute_pressure_pa,
        after.absolute_pressure_pa,
        before.absolute_pressure_pa * expected_t / before.temperature_k
    );
    println!("Snapshot: exact round-trip; momentum remains zero");
    Ok(())
}
