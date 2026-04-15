# UI/UX Design System Guidelines

**Date:** 15-04-2026
**Status:** Active
**Visual Direction:** High-End Developer Tooling (Linear/Vercel Aesthetic)

## 1. Design Philosophy
Hermes is a platform built for speed and precision. The design must be performant and "invisible" – it should never get in the way of the developer's workflow.

* **Hybrid Theme System:** While the platform leans towards a "Dark First" aesthetic, it must support a high-contrast, clean Light Mode.
* **Functional Minimalism:** Remove any decorative elements that don't serve a utility purpose.
* **Data Density:** The UI must allow viewing large volumes of information (logs, metrics) without feeling cluttered.

## 2. Color & Theme System
We use a semantic token system where component colors adapt based on the active theme.

| Token | Dark Mode (Default) | Light Mode |
| :--- | :--- | :--- |
| **Background** | Absolute Black (#0A0A0A) | Pure White (#FFFFFF) |
| **Surface** | Deep Charcoal (#111111) | Soft Gray (#F9F9F9) |
| **Border** | Zinc-800 (#222222) | Zinc-200 (#E4E4E7) |
| **Text Primary** | White (#EDEDED) | Jet Black (#0A0A0A) |
| **Text Muted** | Zinc-400 (#A1A1AA) | Zinc-500 (#71717A) |

### Accent Colors (Shared)
* **Primary:** Electric Blue or Neon Purple for actions and active states.
* **Success:** Emerald Green.
* **Warning:** Amber.
* **Error:** Crimson Red.

## 3. Typography (The "Coding" Vibe)
Using fonts that suggest an engineering and mathematical precision environment.

* **UI Text:** Modern geometric font (Geist or Inter) with high legibility.
* **Technical Data:** Monospaced font (JetBrains Mono) used for IDs, Git branches, Nginx configs, and Environment Variables. This helps the user instantly identify technical configuration from prose.

## 4. Iconography
* **Source:** Google Material Symbols (Rounded variant).
* **Weight:** Light/Thin (weight 300) for an elegant and airy look.
* **Theme Adaptation:** Icons should dim in Dark Mode and gain contrast in Light Mode to maintain visual balance.

## 5. Layout & Spacing
* **Grid System:** All spacing follows 8px increments (Tailwind standard scale).
* **Borders:** In Light Mode, borders become the primary way to define depth. In Dark Mode, we rely on subtle surface color shifts.
* **Interactivity:** Hover transitions are near-instant (150ms) to provide fast tactile feedback.

## 6. Component Behavior
* **Buttons:** Only one primary action per view. Secondary actions are discrete.
* **Inputs:** Focus states must be prominent, using a subtle glow effect (Shadow) in the brand accent color.
* **Loading States:** Use Skeleton Screens that mimic the actual data structure, avoiding generic center-page spinners.
* **Empty States:** Every screen without data must feature a minimalist illustration and a clear Call-to-Action (CTA).