//! Thermodynamic foundation for the new physical-gas backend.
//!
//! This is a spatially uniform, rigid, sealed, adiabatic chamber reference,
//! **not an air-flow solver**. All public physical quantities use SI units.
//! Species masses, planar momentum and total energy are canonical; pressure,
//! density and temperature are derived from them and the occupied volume.
//!
//! The ideal-mixture property model is limited to 200–1000 K. Water remains a
//! noncondensing gas in this reference; phase change, reactions, viscosity and
//! transport are deliberately absent. The constant-heat-capacity alternative
//! is an analytical verification model, not a claim of constant real-air cp.
//!
//! cp/R coefficients: Cantera `data/nasa_gas.yaml`, revision
//! `dbdb9ed525fa2c0066210fda0c285c78cdd3564a`, sourced there from NASA
//! TM-4513 (1993). All five low branches cover 200–1000 K (Ar extends higher).
//! <https://github.com/Cantera/cantera/blob/dbdb9ed525fa2c0066210fda0c285c78cdd3564a/data/nasa_gas.yaml>
//! Only the cp polynomial is used; integration is anchored to the NIST
//! formation enthalpies listed below. This explicitly defines a shared energy
//! zero for both models. Coefficients are scientific data; Cantera is BSD-3
//! licensed. No empirical dense-gas or phase-equilibrium accuracy is claimed.
//! The retained license is in `gas/CANTERA-LICENSE.txt` beside this module.

use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

/// Molar gas constant, J/(mol K), from the exact SI definitions of k_B and N_A.
pub const MOLAR_GAS_CONSTANT: f64 = 8.314_462_618_153_24;
pub const SPECIES_COUNT: usize = 5;
pub const SNAPSHOT_VERSION: u32 = 1;
pub const PROPERTY_DATA_VERSION: &str = "ideal-mixture-nasa7-cp-v1";
pub const REFERENCE_TEMPERATURE_K: f64 = 298.15;
pub const MIN_TEMPERATURE_K: f64 = 200.0;
pub const MAX_TEMPERATURE_K: f64 = 1000.0;

/// Species array order is part of the versioned snapshot format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(usize)]
pub enum Species {
    Nitrogen = 0,
    Oxygen = 1,
    Argon = 2,
    CarbonDioxide = 3,
    WaterVapor = 4,
}

pub const SPECIES: [Species; SPECIES_COUNT] = [
    Species::Nitrogen,
    Species::Oxygen,
    Species::Argon,
    Species::CarbonDioxide,
    Species::WaterVapor,
];

impl Species {
    pub fn molar_mass_kg_per_mol(self) -> f64 {
        // Fixed conventional molecular masses; part of PROPERTY_DATA_VERSION.
        [0.028_013_4, 0.031_998_8, 0.039_948, 0.044_009_5, 0.018_015_28][self as usize]
    }

    /// Standard gas-phase formation enthalpy at 298.15 K, J/mol.
    /// Elemental N2/O2/Ar use zero; CO2 and H2O use NIST Chemistry WebBook
    /// standard values (species IDs C124389 and C7732185, gas phase).
    /// These offsets are already included in stored energy.
    pub fn formation_enthalpy_j_per_mol(self) -> f64 {
        [0.0, 0.0, 0.0, -393_522.4, -241_826.4][self as usize]
    }

    // NASA7 cp/R polynomial a1..a5, low-temperature branch. Integration is
    // anchored to the explicitly stated formation enthalpies above, rather
    // than mixing the fitted enthalpy constants with another energy zero.
    fn cp_coefficients(self) -> [f64; 5] {
        match self {
            Species::Nitrogen => [3.531_005_28, -1.236_609_87e-4, -5.029_994_37e-7, 2.435_306_12e-9, -1.408_812_35e-12],
            Species::Oxygen => [3.782_456_36, -2.996_734_15e-3, 9.847_302e-6, -9.681_295_08e-9, 3.243_728_36e-12],
            Species::Argon => [2.5, 0.0, 0.0, 0.0, 0.0],
            Species::CarbonDioxide => [2.356_773_52, 8.984_596_77e-3, -7.123_562_69e-6, 2.459_190_22e-9, -1.436_995_48e-13],
            Species::WaterVapor => [4.198_640_56, -2.036_434_10e-3, 6.520_402_11e-6, -5.487_970_62e-9, 1.771_978_17e-12],
        }
    }
}

#[derive(Debug, Error)]
pub enum GasError {
    #[error("invalid gas state: {0}")]
    Invalid(&'static str),
    #[error("unsupported gas snapshot version: {0}")]
    UnsupportedVersion(u32),
    #[error("unsupported property data version: {0}")]
    UnsupportedPropertyData(String),
    #[error("gas snapshot encoding failed: {0}")]
    Encode(#[from] toml::ser::Error),
    #[error("gas snapshot decoding failed: {0}")]
    Decode(#[from] toml::de::Error),
}

fn positive_finite(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn validate_temperature(temperature_k: f64) -> Result<(), GasError> {
    if temperature_k.is_finite() && (MIN_TEMPERATURE_K..=MAX_TEMPERATURE_K).contains(&temperature_k) {
        Ok(())
    } else {
        Err(GasError::Invalid("temperature is outside 200–1000 K"))
    }
}

/// Molecular composition, with normalized nonnegative mass fractions.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Composition {
    mass_fractions: [f64; SPECIES_COUNT],
}

impl Composition {
    pub fn from_mass_fractions(fractions: [f64; SPECIES_COUNT]) -> Result<Self, GasError> {
        let value = Self { mass_fractions: fractions };
        value.validate()?;
        Ok(value)
    }

    pub fn from_mole_fractions(fractions: [f64; SPECIES_COUNT]) -> Result<Self, GasError> {
        validate_fractions(&fractions)?;
        let mean_molar_mass: f64 = SPECIES.iter().map(|&s| fractions[s as usize] * s.molar_mass_kg_per_mol()).sum();
        Self::from_mass_fractions(std::array::from_fn(|i| {
            fractions[i] * SPECIES[i].molar_mass_kg_per_mol() / mean_molar_mass
        }))
    }

    /// A specified *dry reference mixture*, not a humidity/ambient model.
    pub fn dry_air_400_ppm_co2() -> Self {
        Self::from_mole_fractions([0.7808, 0.20946, 0.00934, 0.0004, 0.0]).expect("fixed dry-air fractions are valid")
    }

    pub fn mass_fractions(self) -> [f64; SPECIES_COUNT] {
        self.mass_fractions
    }

    pub fn gas_constant_j_per_kg_k(self) -> f64 {
        SPECIES
            .iter()
            .map(|&s| self.mass_fractions[s as usize] * MOLAR_GAS_CONSTANT / s.molar_mass_kg_per_mol())
            .sum()
    }

    pub fn validate(&self) -> Result<(), GasError> {
        validate_fractions(&self.mass_fractions)
    }
}

fn validate_fractions(fractions: &[f64; SPECIES_COUNT]) -> Result<(), GasError> {
    if fractions.iter().any(|x| !x.is_finite() || *x < 0.0) || (fractions.iter().sum::<f64>() - 1.0).abs() > 1e-12 {
        return Err(GasError::Invalid("species fractions must be finite, nonnegative and sum to one"));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThermodynamicModel {
    /// cp(T) from the low-temperature NASA7 polynomial; 200–1000 K only.
    Nasa7,
    /// Freezes each species cv at 298.15 K for analytic verification.
    ConstantCvReference,
}

impl ThermodynamicModel {
    /// Species heat capacity cv in J/(kg K).
    pub fn species_cv_j_per_kg_k(self, species: Species, temperature_k: f64) -> Result<f64, GasError> {
        validate_temperature(temperature_k)?;
        let t = match self {
            Self::Nasa7 => temperature_k,
            Self::ConstantCvReference => REFERENCE_TEMPERATURE_K,
        };
        let a = species.cp_coefficients();
        let cp_over_r = a[0] + t * (a[1] + t * (a[2] + t * (a[3] + t * a[4])));
        Ok((cp_over_r - 1.0) * MOLAR_GAS_CONSTANT / species.molar_mass_kg_per_mol())
    }

    /// Reference plus sensible species internal energy, J/kg. Negative is valid.
    pub fn species_internal_energy_j_per_kg(self, species: Species, temperature_k: f64) -> Result<f64, GasError> {
        validate_temperature(temperature_k)?;
        let reference =
            (species.formation_enthalpy_j_per_mol() - MOLAR_GAS_CONSTANT * REFERENCE_TEMPERATURE_K) / species.molar_mass_kg_per_mol();
        let sensible = match self {
            Self::ConstantCvReference => self.species_cv_j_per_kg_k(species, temperature_k)? * (temperature_k - REFERENCE_TEMPERATURE_K),
            Self::Nasa7 => {
                let a = species.cp_coefficients();
                let delta_h_over_r: f64 = a
                    .iter()
                    .enumerate()
                    .map(|(i, coefficient)| {
                        let n = (i + 1) as i32;
                        coefficient * (temperature_k.powi(n) - REFERENCE_TEMPERATURE_K.powi(n)) / f64::from(n)
                    })
                    .sum();
                MOLAR_GAS_CONSTANT * (delta_h_over_r - (temperature_k - REFERENCE_TEMPERATURE_K)) / species.molar_mass_kg_per_mol()
            }
        };
        Ok(reference + sensible)
    }

    pub fn mixture_cv_j_per_kg_k(self, composition: Composition, temperature_k: f64) -> Result<f64, GasError> {
        composition.validate()?;
        SPECIES
            .iter()
            .map(|&s| {
                self.species_cv_j_per_kg_k(s, temperature_k)
                    .map(|cv| cv * composition.mass_fractions[s as usize])
            })
            .sum()
    }

    fn internal_energy_j(self, species_masses_kg: &[f64; SPECIES_COUNT], temperature_k: f64) -> Result<f64, GasError> {
        SPECIES
            .iter()
            .map(|&s| {
                self.species_internal_energy_j_per_kg(s, temperature_k)
                    .map(|u| species_masses_kg[s as usize] * u)
            })
            .sum()
    }
}

/// Conserved gas inventory. Use validated constructors; raw conserved imports
/// must pass [`GasState::properties`] before they can enter a chamber.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GasState {
    pub species_masses_kg: [f64; SPECIES_COUNT],
    pub momentum_kg_m_per_s: [f64; 2],
    /// Internal (including formation references) plus kinetic energy, J.
    pub total_energy_j: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GasProperties {
    pub mass_kg: f64,
    pub density_kg_per_m3: f64,
    pub temperature_k: f64,
    pub absolute_pressure_pa: f64,
    pub velocity_m_per_s: [f64; 2],
    pub internal_energy_j: f64,
    pub cv_j_per_kg_k: f64,
    pub cp_j_per_kg_k: f64,
}

impl GasState {
    pub fn from_pressure_temperature(
        volume_m3: f64,
        absolute_pressure_pa: f64,
        temperature_k: f64,
        composition: Composition,
        velocity_m_per_s: [f64; 2],
        model: ThermodynamicModel,
    ) -> Result<Self, GasError> {
        composition.validate()?;
        validate_temperature(temperature_k)?;
        if !positive_finite(volume_m3) || !positive_finite(absolute_pressure_pa) || velocity_m_per_s.iter().any(|v| !v.is_finite()) {
            return Err(GasError::Invalid("invalid volume, pressure or velocity"));
        }
        let mass = absolute_pressure_pa * volume_m3 / (composition.gas_constant_j_per_kg_k() * temperature_k);
        let species_masses_kg = composition.mass_fractions.map(|y| y * mass);
        let kinetic_energy = 0.5 * mass * (velocity_m_per_s[0].powi(2) + velocity_m_per_s[1].powi(2));
        let state = Self {
            species_masses_kg,
            momentum_kg_m_per_s: velocity_m_per_s.map(|v| mass * v),
            total_energy_j: model.internal_energy_j(&species_masses_kg, temperature_k)? + kinetic_energy,
        };
        let recovered = state.properties(volume_m3, model)?;
        if (recovered.temperature_k - temperature_k).abs() > 1e-8 * temperature_k {
            return Err(GasError::Invalid("total/kinetic energy precision cannot retain temperature"));
        }
        Ok(state)
    }

    pub fn mass_kg(&self) -> f64 {
        self.species_masses_kg.iter().sum()
    }

    pub fn properties(&self, volume_m3: f64, model: ThermodynamicModel) -> Result<GasProperties, GasError> {
        let mass = self.mass_kg();
        if !positive_finite(volume_m3)
            || !positive_finite(mass)
            || self.species_masses_kg.iter().any(|m| !m.is_finite() || *m < 0.0)
            || self.momentum_kg_m_per_s.iter().any(|p| !p.is_finite())
            || !self.total_energy_j.is_finite()
        {
            return Err(GasError::Invalid("nonfinite or nonphysical conserved inventory"));
        }
        let velocity = self.momentum_kg_m_per_s.map(|p| p / mass);
        let kinetic_energy = 0.5 * mass * (velocity[0].powi(2) + velocity[1].powi(2));
        let internal_energy = self.total_energy_j - kinetic_energy;
        if !internal_energy.is_finite() {
            return Err(GasError::Invalid("nonfinite kinetic/internal energy"));
        }
        let mut lo = MIN_TEMPERATURE_K;
        let mut hi = MAX_TEMPERATURE_K;
        let energy_lo = model.internal_energy_j(&self.species_masses_kg, lo)?;
        let energy_hi = model.internal_energy_j(&self.species_masses_kg, hi)?;
        let thermal_range = energy_hi - energy_lo;
        if !energy_lo.is_finite() || !energy_hi.is_finite() || !positive_finite(thermal_range) {
            return Err(GasError::Invalid("unrepresentable thermal energy range"));
        }
        // E-K can round one ulp beyond a valid endpoint. Permit arithmetic
        // roundoff only, bounded tightly against this inventory's thermal
        // range so an enormous kinetic reservoir cannot excuse invalid U.
        // Canonical energy is never clamped or changed.
        let energy_scale = self
            .total_energy_j
            .abs()
            .max(kinetic_energy.abs())
            .max(energy_lo.abs())
            .max(energy_hi.abs());
        let roundoff = (32.0 * f64::EPSILON * energy_scale).min(1e-10 * thermal_range);
        if internal_energy < energy_lo - roundoff || internal_energy > energy_hi + roundoff {
            return Err(GasError::Invalid("energy implies temperature outside property range"));
        }
        let inversion_energy = internal_energy.clamp(energy_lo, energy_hi);
        // Monotone cv(T)>0 on the declared data interval. Bracketing avoids
        // Newton overshoots and behaves identically for negative reference U.
        for _ in 0..64 {
            let mid = 0.5 * (lo + hi);
            if model.internal_energy_j(&self.species_masses_kg, mid)? < inversion_energy {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let temperature_k = 0.5 * (lo + hi);
        let composition = Composition::from_mass_fractions(self.species_masses_kg.map(|m| m / mass))?;
        let density = mass / volume_m3;
        let gas_constant = composition.gas_constant_j_per_kg_k();
        let pressure = density * gas_constant * temperature_k;
        let cv = model.mixture_cv_j_per_kg_k(composition, temperature_k)?;
        if !positive_finite(density) || !positive_finite(pressure) || !positive_finite(cv) {
            return Err(GasError::Invalid("nonfinite derived thermodynamic state"));
        }
        Ok(GasProperties {
            mass_kg: mass,
            density_kg_per_m3: density,
            temperature_k,
            absolute_pressure_pa: pressure,
            velocity_m_per_s: velocity,
            internal_energy_j: internal_energy,
            cv_j_per_kg_k: cv,
            cp_j_per_kg_k: cv + gas_constant,
        })
    }
}

/// Planar 2D world extruded through a saved, fixed physical slice thickness.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorldMetadata {
    pub version: u32,
    pub backend: GasBackend,
    pub thermodynamic_model: ThermodynamicModel,
    pub width_cells: u32,
    pub height_cells: u32,
    pub fine_cell_length_m: f64,
    pub slice_thickness_m: f64,
    pub ambient_temperature_k: f64,
    pub ambient_absolute_pressure_pa: f64,
    pub ambient_composition: Composition,
    pub gravity_m_per_s2: [f64; 2],
    pub boundaries: ChamberBoundaries,
    pub random_seed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GasBackend {
    UniformSealedChamberReference,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChamberBoundaries {
    AdiabaticImpermeableFreeSlip,
}

impl WorldMetadata {
    /// Dry reference profile, with 1 mm cells, 1 m depth and gravity disabled.
    /// Humid ambient initialization needs an explicit later saturation model.
    pub fn sealed_reference(width_cells: u32, height_cells: u32, model: ThermodynamicModel) -> Self {
        Self {
            version: SNAPSHOT_VERSION,
            backend: GasBackend::UniformSealedChamberReference,
            thermodynamic_model: model,
            width_cells,
            height_cells,
            fine_cell_length_m: 0.001,
            slice_thickness_m: 1.0,
            ambient_temperature_k: 293.15,
            ambient_absolute_pressure_pa: 101_325.0,
            ambient_composition: Composition::dry_air_400_ppm_co2(),
            gravity_m_per_s2: [0.0; 2],
            boundaries: ChamberBoundaries::AdiabaticImpermeableFreeSlip,
            random_seed: 0,
        }
    }

    pub fn volume_m3(&self) -> f64 {
        f64::from(self.width_cells) * f64::from(self.height_cells) * self.fine_cell_length_m.powi(2) * self.slice_thickness_m
    }

    pub fn validate(&self) -> Result<(), GasError> {
        if self.version != SNAPSHOT_VERSION {
            return Err(GasError::UnsupportedVersion(self.version));
        }
        validate_temperature(self.ambient_temperature_k)?;
        self.ambient_composition.validate()?;
        if self.width_cells == 0
            || self.height_cells == 0
            || !positive_finite(self.fine_cell_length_m)
            || !positive_finite(self.slice_thickness_m)
            || !positive_finite(self.volume_m3())
            || !positive_finite(self.ambient_absolute_pressure_pa)
            || self.gravity_m_per_s2 != [0.0; 2]
        {
            return Err(GasError::Invalid("invalid reference geometry/ambient state or nonzero gravity"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceLedger {
    pub initial_species_masses_kg: [f64; SPECIES_COUNT],
    pub initial_total_energy_j: f64,
    /// Signed external heat: positive enters the gas, negative leaves it.
    pub external_heat_j: f64,
    pub accepted_steps: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavedTime {
    seconds: u64,
    nanoseconds: u32,
}

impl From<Duration> for SavedTime {
    fn from(value: Duration) -> Self {
        Self {
            seconds: value.as_secs(),
            nanoseconds: value.subsec_nanos(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Snapshot {
    snapshot_version: u32,
    property_data_version: String,
    metadata: WorldMetadata,
    state: GasState,
    ledger: SourceLedger,
    elapsed: SavedTime,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SealedChamber {
    metadata: WorldMetadata,
    state: GasState,
    ledger: SourceLedger,
    elapsed: Duration,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeatingReport {
    pub applied_heat_j: f64,
    pub accepted_duration: Duration,
    pub before: GasProperties,
    pub after: GasProperties,
}

impl SealedChamber {
    pub fn new(metadata: WorldMetadata) -> Result<Self, GasError> {
        metadata.validate()?;
        let state = GasState::from_pressure_temperature(
            metadata.volume_m3(),
            metadata.ambient_absolute_pressure_pa,
            metadata.ambient_temperature_k,
            metadata.ambient_composition,
            [0.0; 2],
            metadata.thermodynamic_model,
        )?;
        let ledger = SourceLedger {
            initial_species_masses_kg: state.species_masses_kg,
            initial_total_energy_j: state.total_energy_j,
            external_heat_j: 0.0,
            accepted_steps: 0,
        };
        let chamber = Self {
            metadata,
            state,
            ledger,
            elapsed: Duration::ZERO,
        };
        chamber.validate()?;
        Ok(chamber)
    }

    pub fn metadata(&self) -> &WorldMetadata {
        &self.metadata
    }

    pub fn state(&self) -> &GasState {
        &self.state
    }

    pub fn ledger(&self) -> &SourceLedger {
        &self.ledger
    }

    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    pub fn properties(&self) -> Result<GasProperties, GasError> {
        self.state.properties(self.metadata.volume_m3(), self.metadata.thermodynamic_model)
    }

    /// Apply a uniform external heat rate for an *accepted simulated duration*.
    /// A negative rate is cooling. A failed trial changes no state, clock or
    /// ledger, so retrying a shorter duration cannot duplicate source energy.
    pub fn advance_uniform_heater(&mut self, power_w: f64, duration: Duration) -> Result<HeatingReport, GasError> {
        if !power_w.is_finite() || duration.is_zero() {
            return Err(GasError::Invalid("heater requires finite watts and nonzero duration"));
        }
        let before = self.properties()?;
        let heat = power_w * duration.as_secs_f64();
        if !heat.is_finite() {
            return Err(GasError::Invalid("nonfinite integrated heater energy"));
        }
        let mut trial = self.clone();
        trial.state.total_energy_j += heat;
        trial.ledger.external_heat_j += heat;
        trial.ledger.accepted_steps = trial
            .ledger
            .accepted_steps
            .checked_add(1)
            .ok_or(GasError::Invalid("accepted-step counter overflow"))?;
        trial.elapsed = trial
            .elapsed
            .checked_add(duration)
            .ok_or(GasError::Invalid("simulated time overflow"))?;
        trial.validate()?;
        let after = trial.properties()?;
        *self = trial;
        Ok(HeatingReport {
            applied_heat_j: heat,
            accepted_duration: duration,
            before,
            after,
        })
    }

    pub fn validate(&self) -> Result<(), GasError> {
        self.metadata.validate()?;
        self.properties()?;
        let ledger = &self.ledger;
        GasState {
            species_masses_kg: ledger.initial_species_masses_kg,
            momentum_kg_m_per_s: [0.0; 2],
            total_energy_j: ledger.initial_total_energy_j,
        }
        .properties(self.metadata.volume_m3(), self.metadata.thermodynamic_model)?;
        let scale = ledger
            .initial_total_energy_j
            .abs()
            .max(ledger.external_heat_j.abs())
            .max(self.state.total_energy_j.abs())
            .max(1.0);
        if self.state.momentum_kg_m_per_s != [0.0; 2]
            || ledger.initial_species_masses_kg != self.state.species_masses_kg
            || !ledger.initial_total_energy_j.is_finite()
            || !ledger.external_heat_j.is_finite()
            || (ledger.accepted_steps == 0 && (!self.elapsed.is_zero() || ledger.external_heat_j != 0.0))
            || u128::from(ledger.accepted_steps) > self.elapsed.as_nanos()
            || (self.state.total_energy_j - ledger.initial_total_energy_j - ledger.external_heat_j).abs() > 1e-10 * scale
        {
            return Err(GasError::Invalid("sealed chamber inventory/source ledger mismatch"));
        }
        Ok(())
    }

    pub fn to_toml(&self) -> Result<String, GasError> {
        self.validate()?;
        Ok(toml::to_string_pretty(&Snapshot {
            snapshot_version: SNAPSHOT_VERSION,
            property_data_version: PROPERTY_DATA_VERSION.to_owned(),
            metadata: self.metadata.clone(),
            state: self.state.clone(),
            ledger: self.ledger.clone(),
            elapsed: self.elapsed.into(),
        })?)
    }

    pub fn from_toml(source: &str) -> Result<Self, GasError> {
        let snapshot: Snapshot = toml::from_str(source)?;
        if snapshot.snapshot_version != SNAPSHOT_VERSION {
            return Err(GasError::UnsupportedVersion(snapshot.snapshot_version));
        }
        if snapshot.property_data_version != PROPERTY_DATA_VERSION {
            return Err(GasError::UnsupportedPropertyData(snapshot.property_data_version));
        }
        if snapshot.elapsed.nanoseconds >= 1_000_000_000 {
            return Err(GasError::Invalid("noncanonical saved simulation time"));
        }
        let chamber = Self {
            metadata: snapshot.metadata,
            state: snapshot.state,
            ledger: snapshot.ledger,
            elapsed: Duration::new(snapshot.elapsed.seconds, snapshot.elapsed.nanoseconds),
        };
        chamber.validate()?;
        Ok(chamber)
    }
}
