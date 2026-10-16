//! Independent analytic and property references. These verify a uniform
//! thermodynamic system; they are not airflow/transport validation.

use pt_core::gas::{
    Composition, GasState, SealedChamber, Species, ThermodynamicModel, WorldMetadata, MOLAR_GAS_CONSTANT, PROPERTY_DATA_VERSION,
    REFERENCE_TEMPERATURE_K, SPECIES,
};
use std::time::Duration;

fn close(actual: f64, expected: f64, relative: f64, absolute: f64) {
    assert!(
        (actual - expected).abs() <= absolute.max(relative * expected.abs()),
        "actual {actual:.16e}, expected {expected:.16e}"
    );
}

fn chamber(model: ThermodynamicModel) -> SealedChamber {
    SealedChamber::new(WorldMetadata::sealed_reference(1000, 1000, model)).unwrap()
}

#[test]
fn sealed_constant_cv_heating_matches_analytical_energy_temperature_and_pressure() {
    let mut gas = chamber(ThermodynamicModel::ConstantCvReference);
    let initial = gas.properties().unwrap();
    let masses = gas.state().species_masses_kg;
    let energy_before = gas.state().total_energy_j;
    let duration = Duration::from_secs(10);
    let power_w = 1234.5;
    let expected_heat_j = power_w * 10.0;

    // The reference derivative is constant, so U(T2)-U(T1)=m*cv*(T2-T1).
    let expected_t = initial.temperature_k + expected_heat_j / (initial.mass_kg * initial.cv_j_per_kg_k);
    let expected_p = 101_325.0 * expected_t / 293.15;
    let report = gas.advance_uniform_heater(power_w, duration).unwrap();
    let final_state = gas.properties().unwrap();

    assert_eq!(gas.state().species_masses_kg, masses);
    assert_eq!(gas.state().momentum_kg_m_per_s, [0.0; 2]);
    assert_eq!(report.applied_heat_j, expected_heat_j);
    assert_eq!(gas.elapsed(), duration);
    assert_eq!(gas.ledger().accepted_steps, 1);
    close(gas.state().total_energy_j - energy_before, expected_heat_j, 1e-13, 1e-9);
    close(gas.ledger().external_heat_j, expected_heat_j, 1e-14, 1e-12);
    close(final_state.temperature_k, expected_t, 1e-13, 1e-10);
    close(final_state.absolute_pressure_pa, expected_p, 1e-13, 1e-8);
    close(final_state.mass_kg, initial.mass_kg, 0.0, 0.0);
}

#[test]
fn source_budget_and_state_agree_for_equal_simulated_time() {
    let mut batched = chamber(ThermodynamicModel::Nasa7);
    let mut split = batched.clone();
    batched.advance_uniform_heater(1500.0, Duration::from_secs(2)).unwrap();
    for _ in 0..2000 {
        split.advance_uniform_heater(1500.0, Duration::from_millis(1)).unwrap();
    }
    assert_eq!(batched.elapsed(), split.elapsed());
    assert_eq!(batched.state().species_masses_kg, split.state().species_masses_kg);
    assert_eq!(batched.ledger().external_heat_j, split.ledger().external_heat_j);
    close(batched.state().total_energy_j, split.state().total_energy_j, 1e-12, 1e-8);
    close(
        batched.properties().unwrap().temperature_k,
        split.properties().unwrap().temperature_k,
        1e-12,
        1e-9,
    );
}

#[test]
fn no_source_does_not_change_inventories_in_a_thousand_steps() {
    let mut gas = chamber(ThermodynamicModel::Nasa7);
    let original = gas.state().clone();
    for _ in 0..1000 {
        gas.advance_uniform_heater(0.0, Duration::from_millis(1)).unwrap();
    }
    assert_eq!(*gas.state(), original);
    assert_eq!(gas.ledger().external_heat_j, 0.0);
    assert_eq!(gas.elapsed(), Duration::from_secs(1));
}

#[test]
fn rejected_steps_are_transactional_and_a_retry_applies_energy_once() {
    let mut gas = chamber(ThermodynamicModel::Nasa7);
    let original = gas.clone();
    for (watts, duration) in [
        (f64::NAN, Duration::from_secs(1)),
        (f64::INFINITY, Duration::from_secs(1)),
        (100.0, Duration::ZERO),
        (f64::MAX, Duration::from_secs(2)),
        (-1e9, Duration::from_secs(1)),
        (1e9, Duration::from_secs(1)),
    ] {
        assert!(gas.advance_uniform_heater(watts, duration).is_err());
        assert_eq!(gas, original);
    }
    gas.advance_uniform_heater(100.0, Duration::from_secs(1)).unwrap();
    assert_eq!(gas.ledger().external_heat_j, 100.0);
    assert_eq!(gas.ledger().accepted_steps, 1);
    assert_eq!(gas.elapsed(), Duration::from_secs(1));
}

#[test]
fn snapshot_roundtrip_is_exact_and_continuation_is_reproducible() {
    let mut gas = chamber(ThermodynamicModel::Nasa7);
    gas.advance_uniform_heater(97.125, Duration::new(3, 123_456_789)).unwrap();
    let text = gas.to_toml().unwrap();
    let mut restored = SealedChamber::from_toml(&text).unwrap();
    assert_eq!(restored, gas);
    assert_eq!(restored.to_toml().unwrap(), text);
    gas.advance_uniform_heater(-20.0, Duration::from_millis(100)).unwrap();
    restored.advance_uniform_heater(-20.0, Duration::from_millis(100)).unwrap();
    assert_eq!(restored, gas);
}

#[test]
fn snapshots_reject_unsupported_versions_corruption_and_noncanonical_time() {
    let text = chamber(ThermodynamicModel::Nasa7).to_toml().unwrap();
    assert!(text.contains(PROPERTY_DATA_VERSION));
    let invalid_edits: &[(&[&str], toml::Value)] = &[
        (&["snapshot_version"], toml::Value::Integer(99)),
        (&["metadata", "version"], toml::Value::Integer(99)),
        (&["property_data_version"], toml::Value::String("unsupported-data".into())),
        (&["elapsed", "nanoseconds"], toml::Value::Integer(1_000_000_000)),
        (&["ledger", "external_heat_j"], toml::Value::Float(100.0)),
        (
            &["state", "momentum_kg_m_per_s"],
            toml::Value::Array(vec![toml::Value::Float(1.0), toml::Value::Float(0.0)]),
        ),
        (&["metadata", "slice_thickness_m"], toml::Value::Float(0.0)),
        (
            &["state", "species_masses_kg"],
            toml::Value::Array(vec![toml::Value::Float(0.0); 5]),
        ),
    ];
    for (path, replacement) in invalid_edits {
        let mut value: toml::Value = toml::from_str(&text).unwrap();
        let mut field = &mut value;
        for name in *path {
            field = field.get_mut(*name).unwrap();
        }
        *field = replacement.clone();
        let damaged = toml::to_string(&value).unwrap();
        assert!(SealedChamber::from_toml(&damaged).is_err(), "accepted {damaged}");
    }
    assert!(SealedChamber::from_toml(&("unknown_field = 1\n".to_owned() + &text)).is_err());
}

#[test]
fn reference_dependent_negative_energy_is_a_valid_gas_state() {
    let composition = Composition::from_mass_fractions([0.0, 0.0, 0.0, 1.0, 0.0]).unwrap();
    for model in [ThermodynamicModel::Nasa7, ThermodynamicModel::ConstantCvReference] {
        let state = GasState::from_pressure_temperature(1.0, 101_325.0, 293.15, composition, [0.0; 2], model).unwrap();
        assert!(state.total_energy_j < 0.0);
        let derived = state.properties(1.0, model).unwrap();
        close(derived.temperature_k, 293.15, 0.0, 1e-10);
        close(derived.absolute_pressure_pa, 101_325.0, 1e-13, 1e-8);
        // The exact reference h298 is stored once, and u=h-R*T.
        let u = model
            .species_internal_energy_j_per_kg(Species::CarbonDioxide, REFERENCE_TEMPERATURE_K)
            .unwrap();
        close(
            u * Species::CarbonDioxide.molar_mass_kg_per_mol(),
            -393_522.4 - MOLAR_GAS_CONSTANT * REFERENCE_TEMPERATURE_K,
            1e-14,
            1e-9,
        );
    }
}

#[test]
fn momentum_energy_accounting_recovers_the_same_temperature() {
    let composition = Composition::dry_air_400_ppm_co2();
    let model = ThermodynamicModel::Nasa7;
    let stationary = GasState::from_pressure_temperature(0.25, 150_000.0, 450.0, composition, [0.0; 2], model).unwrap();
    let moving = GasState::from_pressure_temperature(0.25, 150_000.0, 450.0, composition, [30.0, -40.0], model).unwrap();
    let actual = moving.properties(0.25, model).unwrap();
    close(actual.temperature_k, 450.0, 0.0, 1e-10);
    close(actual.velocity_m_per_s[0], 30.0, 0.0, 1e-12);
    close(actual.velocity_m_per_s[1], -40.0, 0.0, 1e-12);
    close(
        moving.total_energy_j - stationary.total_energy_j,
        0.5 * moving.mass_kg() * 2500.0,
        1e-13,
        1e-9,
    );
}

#[test]
fn invalid_units_states_and_compositions_are_rejected() {
    assert!(Composition::from_mass_fractions([1.0, 0.1, 0.0, 0.0, 0.0]).is_err());
    assert!(Composition::from_mole_fractions([1.1, -0.1, 0.0, 0.0, 0.0]).is_err());
    assert!(Composition::from_mole_fractions([f64::NAN, 0.0, 0.0, 0.0, 0.0]).is_err());
    let mut metadata = WorldMetadata::sealed_reference(0, 100, ThermodynamicModel::Nasa7);
    assert!(SealedChamber::new(metadata.clone()).is_err());
    metadata.width_cells = 100;
    metadata.gravity_m_per_s2 = [0.0, 9.80665];
    assert!(SealedChamber::new(metadata).is_err());
    let gas = chamber(ThermodynamicModel::Nasa7);
    for volume in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(gas.state().properties(volume, ThermodynamicModel::Nasa7).is_err());
    }
    let mut state = gas.state().clone();
    state.species_masses_kg[0] = -1.0;
    assert!(state.properties(1.0, ThermodynamicModel::Nasa7).is_err());
    state = gas.state().clone();
    state.total_energy_j = f64::NAN;
    assert!(state.properties(1.0, ThermodynamicModel::Nasa7).is_err());
}

#[test]
fn property_range_endpoints_and_variable_heat_capacity_are_supported() {
    for &s in &SPECIES {
        let model = ThermodynamicModel::Nasa7;
        assert!(model.species_cv_j_per_kg_k(s, 200.0).unwrap() > 0.0);
        assert!(model.species_cv_j_per_kg_k(s, 1000.0).unwrap() > 0.0);
        assert!(model.species_cv_j_per_kg_k(s, 199.99).is_err());
        assert!(model.species_cv_j_per_kg_k(s, 1000.01).is_err());
    }
    let model = ThermodynamicModel::Nasa7;
    assert!(
        model.species_cv_j_per_kg_k(Species::Nitrogen, 1000.0).unwrap() > model.species_cv_j_per_kg_k(Species::Nitrogen, 293.15).unwrap()
    );
}

#[test]
fn endpoint_temperatures_survive_kinetic_energy_roundoff() {
    let composition = Composition::from_mole_fractions([1.0, 0.0, 0.0, 0.0, 0.0]).unwrap();
    for model in [ThermodynamicModel::Nasa7, ThermodynamicModel::ConstantCvReference] {
        for temperature in [200.0, 1000.0] {
            let moving = GasState::from_pressure_temperature(
                1.646_541_095_676_121_5,
                101_325.0,
                temperature,
                composition,
                [370.396_611_487_975_8, 0.0],
                model,
            )
            .unwrap();
            close(
                moving.properties(1.646_541_095_676_121_5, model).unwrap().temperature_k,
                temperature,
                0.0,
                1e-10,
            );
            let mut outside = moving;
            outside.total_energy_j += if temperature == 200.0 { -1.0 } else { 1.0 };
            assert!(outside.properties(1.646_541_095_676_121_5, model).is_err());
        }
    }
}

#[test]
fn nasa_cp_and_integrated_energy_match_independent_nist_shomate_references() {
    // Independently evaluated NIST Chemistry WebBook Shomate references,
    // J/(mol K) and J/mol. H2O's Shomate branch starts at 500 K; no room-
    // temperature extrapolation is used. N2 changes branch at 500 K.
    // Source IDs: N2 C7727379, O2 C7782447, Ar C7440371, CO2 C124389,
    // H2O C7732185. https://webbook.nist.gov/cgi/cbook.cgi?ID=C7727379&Units=SI&Mask=1
    // 0.1% is an engineering cross-fit tolerance, not reference uncertainty.
    let cp_500 = [29.58134825, 31.078519, 20.785999959601, 44.614983875, 35.21836175];
    let cp_600 = [30.104019077333, 32.107241226667, 20.786000017498, 47.317934392, 36.317861208889];
    let delta_u = [
        2150.947386451343,
        2328.277525518007,
        1247.153737264442,
        3769.388173442997,
        2744.473301184662,
    ];
    let model = ThermodynamicModel::Nasa7;
    for &species in &SPECIES {
        let i = species as usize;
        let molar_mass = species.molar_mass_kg_per_mol();
        for (temperature, expected) in [(500.0, cp_500[i]), (600.0, cp_600[i])] {
            let cp_molar = model.species_cv_j_per_kg_k(species, temperature).unwrap() * molar_mass + MOLAR_GAS_CONSTANT;
            close(cp_molar, expected, 1e-3, 1e-8);
        }
        let actual_delta_u = (model.species_internal_energy_j_per_kg(species, 600.0).unwrap()
            - model.species_internal_energy_j_per_kg(species, 500.0).unwrap())
            * molar_mass;
        close(actual_delta_u, delta_u[i], 1e-3, 1e-7);
    }
}

#[test]
fn heated_variable_cv_co2_matches_independent_integrated_nist_reference() {
    let mut metadata = WorldMetadata::sealed_reference(1000, 1000, ThermodynamicModel::Nasa7);
    metadata.ambient_temperature_k = 500.0;
    metadata.ambient_composition = Composition::from_mole_fractions([0.0, 0.0, 0.0, 1.0, 0.0]).unwrap();
    let mut gas = SealedChamber::new(metadata).unwrap();
    let initial = gas.properties().unwrap();
    let mol = initial.mass_kg / Species::CarbonDioxide.molar_mass_kg_per_mol();
    // Independent NIST Δu from the previous test; not the model's own integral.
    let heat = mol * 3769.388173442997;
    gas.advance_uniform_heater(heat, Duration::from_secs(1)).unwrap();
    let final_state = gas.properties().unwrap();
    close(final_state.temperature_k, 600.0, 2e-4, 0.0);
    close(final_state.absolute_pressure_pa, 101_325.0 * 600.0 / 500.0, 2e-4, 0.0);
    close(gas.state().total_energy_j - gas.ledger().initial_total_energy_j, heat, 1e-12, 1e-8);
}
