---
description: Implementation plan for 3D immersive server rack visualization on the VM page
---

# 3D Immersive Server Rack Implementation Plan

## Overview

Transform the current CSS-based `Computer3D` component in `site/src/app/vm/page.tsx` into a true 3D immersive space using Three.js and React Three Fiber. The scene will feature a compact server in rack form with three distinct cables connecting to their respective devices:

1. **Network Cable** → Router/Network Hub
2. **Screen Cable** → Monitor displaying GPU output 
3. **UART Cable** → Terminal console displaying terminal output

---

## Phase 1: Dependencies & Project Setup

### 1.1 Install Required Dependencies
```bash
cd /Users/ribo/Projects/personal/havy-os/site
yarn add three @react-three/fiber @react-three/drei
yarn add -D @types/three
```

### 1.2 Create Scene3D Directory Structure
```
site/src/components/Scene3D/
├── index.ts           # Barrel exports
├── Scene.tsx          # Main 3D scene orchestrator  
├── Server.tsx         # Server rack unit with LEDs
├── Monitor.tsx        # Screen display (GPU output)
├── Terminal.tsx       # UART console (terminal output)
├── Router.tsx         # Network device
├── Cables.tsx         # Animated cables connecting devices
├── Environment.tsx    # Lighting, floor, ambient effects
└── CameraController.tsx # Orbit controls & focus points
```

---

## Phase 2: Core 3D Components

### 2.1 Server.tsx - Server Rack Unit
**Visual Design:**
- Small 1U/2U server rack form factor
- Brushed aluminum body with dark accents
- Front panel features:
  - Status LEDs (Power, Activity, Network, Disk)
  - Ventilation grilles with subtle depth
  - Brand label "HAVY VM"
- Rear panel with 3 cable ports (color-coded):
  - Blue port: Network (RJ45 style)
  - Yellow port: Video/Screen (DisplayPort style)
  - Green port: UART/Serial (DB9 style)
- Animated cooling fan mesh visible through grille
- LED states driven by VM status (`isOn`, `status`, `cpuLoad`)

**Props Interface:**
```typescript
interface ServerProps {
  status: VMStatus;
  cpuLoad: number;
  smpInfo: SMPInfo;
  diskInfo: DiskInfo;
  memoryInfo: MemoryInfo;
  onPowerClick: () => void;
}
```

### 2.2 Monitor.tsx - GPU Display Screen
**Visual Design:**
- Modern flat panel monitor with thin bezels
- Stand with cable routing
- Screen displays actual GPU canvas content via texture
- Subtle glow effect when active
- Power LED indicator

**Props Interface:**
```typescript
interface MonitorProps {
  canvasRef: RefObject<HTMLCanvasElement>;
  isActive: boolean;
  width: number;
  height: number;
}
```

**Implementation Notes:**
- Use `THREE.CanvasTexture` to map GPU canvas to screen mesh
- Update texture on each frame via `useFrame()` hook
- Add emissive glow based on screen brightness

### 2.3 Terminal.tsx - UART Console  
**Visual Design:**
- Vintage terminal/console aesthetic
- Thick CRT-style bezel with rounded edges
- Green/amber phosphor text effect on screen
- Terminal displays UART output text
- Physical keyboard attached (optional visual element)

**Props Interface:**
```typescript
interface TerminalProps {
  output: string;
  isActive: boolean;
}
```

**Implementation Notes:**
- Render terminal text to off-screen canvas
- Apply CRT shader effects (scanlines, curvature, glow)
- Map to curved screen geometry

### 2.4 Router.tsx - Network Device
**Visual Design:**
- Small box/router with blinking LEDs
- Network ports on back
- Antenna elements (optional)
- Status LEDs showing network activity

**Props Interface:**
```typescript
interface RouterProps {
  networkActive: boolean;
  dataTransfer: boolean; // For LED blinking
}
```

### 2.5 Cables.tsx - Animated Cable System
**Visual Design:**
- 3 distinct cables with realistic catenary curves
- Color-coded by function:
  - **Blue**: Network cable (Ethernet style)
  - **Yellow**: Video cable (thick, shielded)
  - **Green**: UART/Serial cable (thinner)
- Animated data flow particles along cables when active
- Subtle physics-based sway animation
- Connectors at each end matching port style

**Props Interface:**
```typescript
interface CablesProps {
  serverPosition: Vector3;
  monitorPosition: Vector3;
  terminalPosition: Vector3;
  routerPosition: Vector3;
  networkActive: boolean;
  screenActive: boolean;
  uartActive: boolean;
}
```

**Implementation Notes:**
- Use `THREE.TubeGeometry` with `CatmullRomCurve3` for smooth curves
- Particle system for data flow effect
- Shader-based glow on active cables

### 2.6 Environment.tsx - Scene Environment
**Visual Design:**
- Dark, professional server room aesthetic
- Subtle reflective floor (like polished concrete)
- Ambient fog for depth
- Volumetric lighting accents
- Background: dark gradient or subtle grid pattern

**Implementation:**
```typescript
// Lighting setup
- Soft area light from above
- Accent rim lighting for object definition  
- Subtle blue ambient for tech feel
- Point lights from LED sources

// Effects
- Subtle bloom on LEDs and screens
- Depth of field (optional)
- Vignette for focus
```

### 2.7 CameraController.tsx - Navigation
**Features:**
- Orbit controls with damping
- Click-to-focus on devices (server, monitor, terminal, router)
- Mouse wheel zoom with limits
- Auto-rotate when idle (optional)
- Smooth animated transitions between focus points

**Implementation:**
```typescript
interface CameraControllerProps {
  focusTarget: 'overview' | 'server' | 'monitor' | 'terminal' | 'router';
  onFocusChange: (target: string) => void;
}
```

---

## Phase 3: Scene Integration

### 3.1 Scene.tsx - Main Orchestrator
**Responsibilities:**
- Canvas and WebGL context management
- Component positioning and layout
- State distribution to child components
- Performance optimization (LOD, frustum culling)

**Scene Layout (Top View):**
```
                    [Router]
                       |
                       | (network cable)
                       |
    [Terminal] ----[SERVER]---- [Monitor]
       (uart)                    (video)
```

**Approximate Positions:**
- Server: Origin (0, 0, 0)
- Monitor: Right (+3, 0, 0)
- Terminal: Left (-3, 0, 0)  
- Router: Back (0, 0, -2)

**Props Interface:**
```typescript
interface Scene3DProps {
  // VM State
  status: VMStatus;
  output: string;
  cpuLoad: number;
  smpInfo: SMPInfo;
  diskInfo: DiskInfo;
  memoryInfo: MemoryInfo;
  
  // GPU Display
  gpuCanvasRef: RefObject<HTMLCanvasElement>;
  gpuActive: boolean;
  gpuSize: { width: number; height: number };
  
  // Controls  
  onPowerClick: () => void;
  
  // Config
  enableNetwork: boolean;
  enableGPU: boolean;
}
```

---

## Phase 4: VM Page Integration

### 4.1 Update page.tsx
**Changes Required:**

1. **Import Scene3D component:**
```typescript
import { Scene3D } from "../../components/Scene3D";
```

2. **Replace Computer3D with Scene3D:**
```tsx
// Before
<Computer3D {...props}>
  {screenContent}
</Computer3D>

// After  
<Scene3D
  status={status}
  output={output}
  cpuLoad={cpuLoad}
  smpInfo={smpInfo}
  diskInfo={diskInfo}
  memoryInfo={memoryInfo}
  gpuCanvasRef={canvasRef}
  gpuActive={gpuActive}
  gpuSize={gpuSize}
  onPowerClick={handlePowerClick}
  enableNetwork={vmConfig.enableNetwork}
  enableGPU={vmConfig.enableGPU}
/>
```

3. **Remove legacy Computer3D component** (lines ~305-389)

4. **Keep VMConfigPanel** - Display as overlay or integrate into 3D scene

5. **Update CSS** - Add new styles for 3D canvas container, overlays

---

## Phase 5: Advanced Features

### 5.1 Interactive Elements
- Clickable power button on server
- Click device to focus camera
- Hover tooltips showing status info
- Keyboard shortcuts (1-4 for quick focus)

### 5.2 Visual Feedback
- Cable pulse animation on data transfer
- LED brightness based on CPU load
- Screen flicker on boot
- Cooling fan speed based on load

### 5.3 Performance Optimizations
- Instanced geometry for repeated elements
- LOD (Level of Detail) for distant objects
- Lazy loading of textures
- Frame limiting when idle
- `<Suspense>` for async loading states

### 5.4 Accessibility
- Keyboard navigation
- Screen reader descriptions
- Reduced motion option
- High contrast mode

---

## Phase 6: Polish & Effects

### 6.1 Post-Processing
```typescript
import { EffectComposer, Bloom, Vignette } from '@react-three/postprocessing';

<EffectComposer>
  <Bloom luminanceThreshold={0.8} intensity={0.5} />
  <Vignette offset={0.3} darkness={0.5} />
</EffectComposer>
```

### 6.2 Shader Effects
- CRT effect for terminal screen
- Holographic labels (optional)
- Heat distortion over server (optional)
- Cable data flow particles

---

## Implementation Order

1. ✅ **Phase 1**: Install dependencies, create directory structure
2. ⬜ **Phase 2.6**: Environment.tsx (lighting, floor)
3. ⬜ **Phase 2.1**: Server.tsx (core element)
4. ⬜ **Phase 2.5**: Cables.tsx (visual connections)
5. ⬜ **Phase 2.2**: Monitor.tsx (GPU display)
6. ⬜ **Phase 2.3**: Terminal.tsx (UART output)
7. ⬜ **Phase 2.4**: Router.tsx (network device)
8. ⬜ **Phase 2.7**: CameraController.tsx (navigation)
9. ⬜ **Phase 3.1**: Scene.tsx (orchestrator)
10. ⬜ **Phase 4.1**: Integrate into page.tsx
11. ⬜ **Phase 5**: Interactive features
12. ⬜ **Phase 6**: Polish & post-processing

---

## File Size Estimates

| Component | Lines | Complexity |
|-----------|-------|------------|
| Server.tsx | ~200 | High |
| Monitor.tsx | ~150 | Medium |
| Terminal.tsx | ~180 | High |
| Router.tsx | ~100 | Medium |
| Cables.tsx | ~250 | High |
| Environment.tsx | ~80 | Low |
| CameraController.tsx | ~120 | Medium |
| Scene.tsx | ~150 | Medium |
| **Total** | **~1230** | - |

---

## Testing Checklist

- [ ] Scene loads without WebGL errors
- [ ] GPU canvas texture updates smoothly (60fps)
- [ ] Terminal text renders correctly
- [ ] Cables animate properly
- [ ] LEDs respond to VM status changes
- [ ] Camera controls feel smooth
- [ ] Mobile touch controls work
- [ ] No performance regression (maintain 60fps)
- [ ] Graceful fallback for older browsers

---

## Rollback Plan

If issues arise, the existing `Computer3D` CSS component remains in the codebase. The change is isolated to:
1. New files in `components/Scene3D/`
2. Import changes in `app/vm/page.tsx`

Rollback: Revert the import in page.tsx to use `Computer3D` again.
