# Laws

Every law function in `gx-core`, its units, and its determinism contract. The laws produce numbers for the renderer; they never produce hub bytes, but they are held to the determinism rule of `determinism.md` so a renderer and a conformance check agree bit for bit. No law knows what matter is: each reads only positions, masses, temperatures, and the optical channels of `matter-format.md` section 3.3.

## Bands

Format version 1 fixes three wavelength bands, `gx_core::radiance::BAND_EDGES = [700e-9, 600e-9, 500e-9, 400e-9]` meters, long to short wavelength:

| Band | From | To |
|---|---|---|
| 0 | 600 nm | 700 nm |
| 1 | 500 nm | 600 nm |
| 2 | 400 nm | 500 nm |

The three albedo values of a sample map to these bands in this order. Every per-band value below (`[f64; 3]`) is band 0 first.

## Constants

| Constant | Value | Unit | Source |
|---|---|---|---|
| `gravity::G` | 6.67430e-11 | m^3 kg^-1 s^-2 | CODATA 2018 |
| `radiance::PLANCK` | 6.62607015e-34 | J s | CODATA 2018, exact |
| `radiance::SPEED_OF_LIGHT` | 299792458 | m/s | CODATA 2018, exact |
| `radiance::BOLTZMANN` | 1.380649e-23 | J/K | CODATA 2018, exact |
| `radiance::STEFAN_BOLTZMANN` | 5.670374419e-8 | W m^-2 K^-4 | CODATA 2018 |

## Functions

| Function | Inputs | Output | Determinism |
|---|---|---|---|
| `gravity::acceleration_at` | point (m), point masses (m, kg) | m/s^2 | sum in slice order |
| `gravity::potential_at` | point (m), point masses (m, kg) | J/kg | sum in slice order |
| `gravity::acceleration_from_section` | point (m), section | m/s^2 | sum in sample index order |
| `integrate::advance` | frame system, target time (s) | frame states | fixed step count, see `determinism.md` |
| `radiance::spectral_radiance` | wavelength (m), temperature (K) | W m^-2 sr^-1 m^-1 | written-out exponential |
| `radiance::band_radiance` | temperature (K) | W m^-2 sr^-1 per band | 64-point midpoint rule per band, ascending wavelength |
| `radiance::total_radiance` | temperature (K) | W m^-2 sr^-1 | closed form `sigma T^4 / pi` |
| `radiance::emitted_band_radiance` | temperature (K), albedo per band | W m^-2 sr^-1 per band | `band_radiance * (1 - albedo)` |
| `emission::summarize` | section, minimum temperature (K) | position (m), power per band (W), radius (m) | sums in sample index order |
| `extinction::extinction_coefficient` | density (kg/m^3), attenuation (m^2/kg) | 1/m | one product |
| `extinction::transmittance` | density, attenuation, path (m) | ratio | written-out exponential |
| `extinction::optical_depth_along` | section, segment ends (m), steps | dimensionless | caller-fixed steps, summed in order |
| `lod::select_cells` | frame, camera (m), selection params | cell keys | fixed refinement order, sorted output |

### Radiance

`spectral_radiance` is Planck's law, `2 h c^2 / lambda^5 / (exp(h c / (lambda k_B T)) - 1)`. It returns 0 for a temperature or wavelength at or below 0 and when the exponent exceeds 700, so cold matter at short wavelength never overflows.

`band_radiance` integrates it over each band with a 64-point midpoint rule in `f64`: `width = (hi - lo) / 64`, then the sum over `j` from 0 to 63 of `spectral_radiance(lo + (j + 0.5) * width) * width`. **The rule and the point count are part of the contract.** Changing either changes every band value.

`emitted_band_radiance` applies Kirchhoff's law for opaque matter: emissivity is `1 - albedo` per band.

### Emission

`summarize` reduces every non-vacuum sample at or above a minimum temperature to one emitter: the power-weighted centroid of the sample centers, the power per band in watts, and the unweighted root mean square distance of the sample centers from the centroid. A sample's power in a band is `emitted_band_radiance * pi * 6 * s^2` for sub-cube edge `s`: hemispherical radiance over the six faces of the sub-cube. This is a deliberate v1 approximation that ignores faces hidden by hot neighbours. Returns `None` when no sample qualifies.

### Extinction

`extinction_coefficient` is `density * attenuation`, per meter. `transmittance` is `exp(-k * path)`. `optical_depth_along` samples the midpoints of `steps` equal parts of a segment, looks up the sample whose sub-cube holds each midpoint, and sums `k * part length`; midpoints outside the cell add nothing.

### Cell selection

`select_cells` walks a frame's octree from depth 0. A cell's projected edge in pixels is `edge / max(distance, edge) * view_height_px / (2 tan(fov / 2))`, with `distance` from the camera to the nearest point of the cell. A cell is refined into its eight children while that exceeds `pixel_error` and its depth is below the frame's `max_depth`. Cells farther than `4 * root_extent` are culled. Refinement runs largest projected edge first (ties by ascending `(depth, x, y, z)`) and stops as soon as the next refinement would exceed `max_cells`, so the result never holds more than `max_cells` cells. The output is sorted by `(depth, x, y, z)`.

## How the determinism is kept

- Only `+`, `-`, `*`, `/`, `sqrt`, `round`, and `floor`-like truncation, all exact or correctly rounded in IEEE 754.
- `exp`, `expm1`, and `tan` are written out in a private module (`detmath`): range reduction by an exactly split `ln 2`, a fixed-length Taylor series in Horner form, and scaling by an exact power of two. The platform math library is never called by a law.
- Every sum runs in a fixed order with plain operators: no fused multiply add, no SIMD reductions, no hash map iteration.
