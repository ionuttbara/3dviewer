#[derive(Clone, Debug)]
pub struct Settings {
    pub theme_mode: i32,
    pub filament_price_per_kg: f32,
    pub filament_density_g_cm3: f32,
    pub infill_percent: f32,
    pub cad_linear_deflection_mm: f32,
    pub cad_angular_deflection_deg: f32,
    pub cad_relative_deflection: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self { theme_mode: 2, filament_price_per_kg: 60.0, filament_density_g_cm3: 1.24, infill_percent: 20.0, cad_linear_deflection_mm: 0.10, cad_angular_deflection_deg: 20.0, cad_relative_deflection: false }
    }
}

#[cfg(windows)]
impl Settings {
    pub fn load() -> Self {
        use winreg::{enums::KEY_READ, HKCU};
        let mut s = Self::default();
        // 0.2.2 display name is "3D Viewer". Read the new key first, then fall
        // back to the pre-0.2.2 key so upgrades keep existing preferences.
        let key = HKCU.open_subkey_with_flags("Software\\Gallery Inc\\3D Viewer", KEY_READ)
            .or_else(|_| HKCU.open_subkey_with_flags("Software\\Gallery Inc\\Gallery 3D Viewer", KEY_READ));
        if let Ok(k) = key {
            s.theme_mode = k.get_value::<u32,_>("ThemeMode").unwrap_or(s.theme_mode as u32) as i32;
            s.filament_price_per_kg = f32::from_bits(k.get_value::<u32,_>("FilamentPriceBits").unwrap_or(s.filament_price_per_kg.to_bits()));
            s.filament_density_g_cm3 = f32::from_bits(k.get_value::<u32,_>("FilamentDensityBits").unwrap_or(s.filament_density_g_cm3.to_bits()));
            s.infill_percent = f32::from_bits(k.get_value::<u32,_>("InfillBits").unwrap_or(s.infill_percent.to_bits()));
            s.cad_linear_deflection_mm = f32::from_bits(k.get_value::<u32,_>("CadLinearDeflectionBits").unwrap_or(s.cad_linear_deflection_mm.to_bits())).clamp(0.001, 10.0);
            s.cad_angular_deflection_deg = f32::from_bits(k.get_value::<u32,_>("CadAngularDeflectionBits").unwrap_or(s.cad_angular_deflection_deg.to_bits())).clamp(0.5, 90.0);
            s.cad_relative_deflection = k.get_value::<u32,_>("CadRelativeDeflection").unwrap_or(s.cad_relative_deflection as u32) != 0;
        }
        s
    }

    pub fn save(&self) {
        use winreg::HKCU;
        if let Ok((k,_)) = HKCU.create_subkey("Software\\Gallery Inc\\3D Viewer") {
            let _ = k.set_value("ThemeMode", &(self.theme_mode as u32));
            let _ = k.set_value("FilamentPriceBits", &self.filament_price_per_kg.to_bits());
            let _ = k.set_value("FilamentDensityBits", &self.filament_density_g_cm3.to_bits());
            let _ = k.set_value("InfillBits", &self.infill_percent.to_bits());
            let _ = k.set_value("CadLinearDeflectionBits", &self.cad_linear_deflection_mm.to_bits());
            let _ = k.set_value("CadAngularDeflectionBits", &self.cad_angular_deflection_deg.to_bits());
            let _ = k.set_value("CadRelativeDeflection", &(self.cad_relative_deflection as u32));
        }
    }
}

#[cfg(not(windows))]
impl Settings { pub fn load() -> Self { Self::default() } pub fn save(&self) {} }
