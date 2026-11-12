import Cocoa

// Сугроб — снег поверх рабочего стола macOS.
// Координаты симуляции: точки, начало в левом верхнем углу основного экрана, y вниз
// (так же, как отдаёт CGWindowList).

let colW: CGFloat = 2          // ширина столбика сугроба
let maxPile: CGFloat = 18      // предел высоты сугроба
let slope: CGFloat = 1.4       // макс. перепад между соседними столбиками (угол осыпания)
let cursorR: CGFloat = 12      // радиус "лопаты" курсора
let maxParticles = 6000

func rnd(_ a: CGFloat, _ b: CGFloat) -> CGFloat { CGFloat.random(in: a...b) }

struct Particle {
    var x, y, vx, vy, r, mass: CGFloat
    var land: Bool      // может ложиться на поверхности
    var debris: Bool    // комок, сбитый с сугроба (тяжёлый, падает быстро)
    var layer: Int      // 0 дальний … 2 ближний
    var phase: CGFloat
}

final class Surface {
    var frame: CGRect
    var x0: CGFloat
    var y: CGFloat
    var h: [CGFloat]
    var vis: [Bool]
    var seen = true
    var spillL: CGFloat = 0, spillR: CGFloat = 0
    init(frame: CGRect, x0: CGFloat, y: CGFloat, n: Int) {
        self.frame = frame; self.x0 = x0; self.y = y
        h = Array(repeating: 0, count: n); vis = Array(repeating: true, count: n)
    }
}

final class Sim {
    var W: CGFloat = 0, H: CGFloat = 0
    var particles: [Particle] = []
    var surfaces: [Int: Surface] = [:]
    var rate: CGFloat = 40          // снежинок в секунду на 1000 pt ширины
    var t: CGFloat = 0
    var spawnAcc: CGFloat = 0
    let myPid = Int(ProcessInfo.processInfo.processIdentifier)

    // MARK: частицы

    func debris(_ x: CGFloat, _ y: CGFloat, _ vx: CGFloat, _ vy: CGFloat, _ mass: CGFloat) {
        guard particles.count < maxParticles, mass > 0.05 else { return }
        let r = min(3.5, max(1, (mass / 2).squareRoot()))
        particles.append(Particle(x: x, y: y, vx: vx, vy: vy, r: r, mass: mass,
                                  land: true, debris: true, layer: 2, phase: 0))
    }

    func dump(_ s: Surface, vel: CGPoint) {
        for i in 0..<s.h.count where s.h[i] > 0.3 {
            let mass = s.h[i] * colW
            let k = max(1, Int(mass / 8))
            let x = s.x0 + (CGFloat(i) + 0.5) * colW
            for _ in 0..<k {
                debris(x + rnd(-1, 1), s.y - rnd(0, s.h[i]),
                       vel.x * 0.4 + rnd(-25, 25), vel.y * 0.4 + rnd(-15, 25), mass / CGFloat(k))
            }
            s.h[i] = 0
        }
    }

    func dumpAll() { for s in surfaces.values { dump(s, vel: .zero) } }

    func deposit(_ s: Surface, _ c: Int, _ mass: CGFloat) {
        let dh = mass / colW
        let n = s.h.count
        let l = (c > 0 && s.vis[c - 1]) ? c - 1 : c
        let r = (c < n - 1 && s.vis[c + 1]) ? c + 1 : c
        s.h[c] += dh * 0.5; s.h[l] += dh * 0.25; s.h[r] += dh * 0.25
    }

    // MARK: окна

    func upsert(_ id: Int, frame: CGRect, xA: CGFloat, xB: CGFloat, y: CGFloat,
                occ: [CGRect], pollDt: CGFloat) {
        let n = Int((xB - xA) / colW)
        guard n > 4 else { return }
        var s = surfaces[id]
        if let old = s {
            let dx = frame.minX - old.frame.minX, dy = frame.minY - old.frame.minY
            if abs(dx) > 0.5 || abs(dy) > 0.5 || abs(frame.width - old.frame.width) > 0.5 || old.h.count != n {
                // окно сдвинули / растянули — снег стряхивается
                var v = CGPoint(x: dx / pollDt, y: dy / pollDt)
                v.x = max(-800, min(800, v.x)); v.y = max(-800, min(800, v.y))
                dump(old, vel: v)
                s = nil
            }
        }
        let surf = s ?? Surface(frame: frame, x0: xA, y: y, n: n)
        surf.seen = true
        for i in 0..<n {
            let x = xA + (CGFloat(i) + 0.5) * colW
            let v = x >= 0 && x < W && !occ.contains {
                $0.minX <= x && x < $0.maxX && $0.minY < y - 1 && $0.maxY > y - 1
            }
            surf.vis[i] = v
            if !v { surf.h[i] = 0 }
        }
        surfaces[id] = surf
    }

    func refreshWindows(dock: CGRect?, pollDt: CGFloat) {
        for s in surfaces.values { s.seen = false }
        var occ: [CGRect] = []
        let opts: CGWindowListOption = [.optionOnScreenOnly, .excludeDesktopElements]
        let list = CGWindowListCopyWindowInfo(opts, kCGNullWindowID) as? [[String: Any]] ?? []
        for w in list {   // порядок: от переднего окна к заднему
            guard (w[kCGWindowLayer as String] as? Int) == 0,
                  (w[kCGWindowOwnerPID as String] as? Int) != myPid,
                  (w[kCGWindowAlpha as String] as? Double ?? 1) > 0.05,
                  let bd = w[kCGWindowBounds as String] as? NSDictionary,
                  let b = CGRect(dictionaryRepresentation: bd as CFDictionary) else { continue }
            if b.width >= 160, b.height >= 80, b.minY > 5, b.minY < H - 20, b.maxX > 0, b.minX < W,
               let id = w[kCGWindowNumber as String] as? Int {
                // отступ 8 pt — скруглённые углы окна
                upsert(id, frame: b, xA: b.minX + 8, xB: b.maxX - 8, y: b.minY, occ: occ, pollDt: pollDt)
            }
            if b.width >= 50, b.height >= 50 { occ.append(b) }
        }
        var groundOcc: [CGRect] = []
        if let d = dock {
            upsert(-1, frame: d, xA: d.minX, xB: d.maxX, y: d.minY - 5, occ: [], pollDt: pollDt)
            groundOcc = [CGRect(x: d.minX - 12, y: 0, width: d.width + 24, height: H + 10)]
        }
        upsert(0, frame: CGRect(x: 0, y: H, width: W, height: 1), xA: 0, xB: W, y: H,
               occ: groundOcc, pollDt: pollDt)
        for (id, s) in surfaces where !s.seen {   // окно закрыли — снег падает
            dump(s, vel: .zero)
            surfaces[id] = nil
        }
    }

    // MARK: шаг физики

    func step(dt: CGFloat, c: CGPoint, cv: CGPoint) {
        t += dt
        // новые снежинки
        spawnAcc += rate * dt * W / 1000
        while spawnAcc >= 1 {
            spawnAcc -= 1
            guard particles.count < maxParticles else { continue }
            let u = rnd(0, 1)
            let r = 1 + 2.2 * u * u
            let layer = r < 1.5 ? 0 : (r < 2.2 ? 1 : 2)
            particles.append(Particle(x: rnd(-60, W + 60), y: -8, vx: 0, vy: 20 + r * 15, r: r,
                                      mass: r * r * 2, land: layer > 0 && rnd(0, 1) < 0.7,
                                      debris: false, layer: layer, phase: rnd(0, 6.28)))
        }

        let wind = 25 * sin(t * 0.13) + 12 * sin(t * 0.51 + 1)
        let surfs = Array(surfaces.values)

        // курсор сгребает сугробы
        for s in surfs where c.y > s.y - maxPile - cursorR - 4 && c.y < s.y + cursorR {
            let a = max(0, Int((c.x - cursorR - s.x0) / colW))
            let b = min(s.h.count - 1, Int((c.x + cursorR - s.x0) / colW))
            guard a <= b else { continue }
            for i in a...b where s.h[i] > 0.3 {
                let x = s.x0 + (CGFloat(i) + 0.5) * colW
                let dx = x - c.x
                guard abs(dx) < cursorR else { continue }
                let sh = (cursorR * cursorR - dx * dx).squareRoot()
                guard c.y - sh < s.y else { continue }
                let allowed = max(0, s.y - (c.y + sh))
                guard s.h[i] > allowed + 0.3 else { continue }
                let ex = s.h[i] - allowed
                s.h[i] = allowed
                let mass = ex * colW
                let k = min(3, max(1, Int(mass / 5)))
                for _ in 0..<k {
                    debris(x, s.y - allowed - rnd(0, ex),
                           cv.x * 0.6 + dx / cursorR * 70 + rnd(-50, 50),
                           -abs(cv.y) * 0.3 - rnd(30, 130), mass / CGFloat(k))
                }
            }
        }

        // частицы
        var i = 0
        while i < particles.count {
            var p = particles[i]
            let prevY = p.y
            let term: CGFloat = p.debris ? 460 : 28 + p.r * 20
            let k: CGFloat = p.debris ? 2.2 : 3
            let tx = p.debris ? wind * 0.3 : wind * (0.5 + p.r * 0.25) + 14 * sin(t * 1.7 + p.phase)

            // вихрь от курсора
            let dx = p.x - c.x, dy = p.y - c.y
            let d2 = dx * dx + dy * dy
            if d2 < 4900 {
                let d = max(1, d2.squareRoot())
                let f = 1 - d / 70
                let g = min(1, f * 8 * dt)
                p.vx += (cv.x * 0.6 - p.vx) * g + dx / d * f * 260 * dt
                p.vy += (cv.y * 0.6 - p.vy) * g + dy / d * f * 260 * dt
            }
            p.vx += (tx - p.vx) * k * dt
            p.vy += (term - p.vy) * k * dt
            p.x += p.vx * dt
            p.y += p.vy * dt

            var dead = p.y > H + 12 || p.x < -80 || p.x > W + 80
            if !dead && p.land && p.vy > 0 {
                for s in surfs where p.y >= s.y - maxPile - 30 && prevY <= s.y + 2 {
                    let col = Int(((p.x - s.x0) / colW).rounded(.down))
                    guard col >= 0, col < s.h.count, s.vis[col] else { continue }
                    let top = s.y - s.h[col]
                    guard prevY <= top + 1.5, p.y >= top else { continue }
                    let fill = s.h[col] / maxPile
                    if p.debris || rnd(0, 1) > fill * fill {
                        deposit(s, col, p.mass)
                        dead = true
                    }
                    break
                }
            }
            if dead {
                particles.swapAt(i, particles.count - 1)
                particles.removeLast()
            } else {
                particles[i] = p
                i += 1
            }
        }

        // осыпание сугробов и срыв снега с краёв
        for s in surfs {
            let n = s.h.count
            for pass in 0..<2 {
                for j in 0..<(n - 1) {
                    let a = pass == 0 ? j : n - 2 - j
                    guard s.vis[a], s.vis[a + 1] else { continue }
                    let d = s.h[a] - s.h[a + 1]
                    if d > slope { let m = (d - slope) * 0.25; s.h[a] -= m; s.h[a + 1] += m }
                    else if d < -slope { let m = (-d - slope) * 0.25; s.h[a] += m; s.h[a + 1] -= m }
                }
            }
            guard s.y < H - 1 else { continue }   // у земли краёв нет
            if s.vis[0], s.h[0] > 2 {
                let m = (s.h[0] - 2) * 0.2
                s.h[0] -= m; s.spillL += m * colW
                while s.spillL >= 4 {
                    s.spillL -= 4
                    debris(s.x0 - 1, s.y - rnd(0, s.h[0]), rnd(-30, -6), rnd(-10, 10), 4)
                }
            }
            if s.vis[n - 1], s.h[n - 1] > 2 {
                let m = (s.h[n - 1] - 2) * 0.2
                s.h[n - 1] -= m; s.spillR += m * colW
                while s.spillR >= 4 {
                    s.spillR -= 4
                    debris(s.x0 + CGFloat(n) * colW + 1, s.y - rnd(0, s.h[n - 1]), rnd(6, 30), rnd(-10, 10), 4)
                }
            }
        }
    }

    // MARK: геометрия для отрисовки

    func flakePaths() -> [CGPath] {
        let paths = [CGMutablePath(), CGMutablePath(), CGMutablePath()]
        for p in particles {
            paths[p.layer].addEllipse(in: CGRect(x: p.x - p.r, y: p.y - p.r, width: p.r * 2, height: p.r * 2))
        }
        return paths
    }

    func pilePath() -> CGPath {
        let path = CGMutablePath()
        for s in surfaces.values {
            let n = s.h.count
            var i = 0
            while i < n {
                guard s.vis[i], s.h[i] > 0.15 else { i += 1; continue }
                var j = i
                while j < n, s.vis[j], s.h[j] > 0.15 { j += 1 }
                path.move(to: CGPoint(x: s.x0 + CGFloat(i) * colW, y: s.y + 0.5))
                for k in i..<j {
                    let l = k > i ? s.h[k - 1] : 0, r = k < j - 1 ? s.h[k + 1] : 0
                    path.addLine(to: CGPoint(x: s.x0 + (CGFloat(k) + 0.5) * colW,
                                             y: s.y - (l + 2 * s.h[k] + r) / 4))
                }
                path.addLine(to: CGPoint(x: s.x0 + CGFloat(j) * colW, y: s.y + 0.5))
                path.closeSubpath()
                i = j
            }
        }
        return path
    }
}

// MARK: - приложение

final class AppDelegate: NSObject, NSApplicationDelegate, NSMenuDelegate {
    let sim = Sim()
    var window: NSWindow!
    var flakeLayers: [CAShapeLayer] = []
    let pileLayer = CAShapeLayer()
    let root = CALayer()
    var link: CADisplayLink?
    var statusItem: NSStatusItem!
    var lastTime: CFTimeInterval = 0
    var lastMouse = CGPoint.zero
    var mouseVel = CGPoint.zero
    var pollAcc: CGFloat = 1
    var dockAcc: CGFloat = 10
    var dockRect: CGRect?
    var running = true
    let rates: [(String, CGFloat)] = [("Лёгкий снежок", 15), ("Снегопад", 40), ("Метель", 110)]

    func applicationDidFinishLaunching(_ note: Notification) {
        let saved = UserDefaults.standard.double(forKey: "rate")
        if saved > 0 { sim.rate = CGFloat(saved) }

        let screen = NSScreen.screens[0]
        window = NSWindow(contentRect: screen.frame, styleMask: .borderless, backing: .buffered, defer: false)
        window.isOpaque = false
        window.backgroundColor = .clear
        window.hasShadow = false
        window.ignoresMouseEvents = true
        window.level = .statusBar
        window.collectionBehavior = [.canJoinAllSpaces, .stationary, .ignoresCycle, .fullScreenAuxiliary]

        let view = NSView(frame: screen.frame)
        let host = CALayer()
        view.layer = host
        view.wantsLayer = true
        window.contentView = view
        // Симуляция считает y вниз, слой AppKit — y вверх: переворачиваем явно.
        host.addSublayer(root)

        let white = CGColor(red: 1, green: 1, blue: 1, alpha: 1)
        for alpha in [0.45, 0.7, 0.95] as [Float] {
            let l = CAShapeLayer()
            l.fillColor = white
            l.opacity = alpha
            root.addSublayer(l)
            flakeLayers.append(l)
        }
        pileLayer.fillColor = white
        pileLayer.shadowColor = CGColor(red: 0.1, green: 0.2, blue: 0.4, alpha: 1)
        pileLayer.shadowOpacity = 0.35
        pileLayer.shadowRadius = 1.5
        pileLayer.shadowOffset = .zero
        root.addSublayer(pileLayer)
        layout()
        window.orderFrontRegardless()

        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.variableLength)
        statusItem.button?.title = "❄️"
        let menu = NSMenu()
        menu.delegate = self
        statusItem.menu = menu

        NotificationCenter.default.addObserver(self, selector: #selector(layout),
            name: NSApplication.didChangeScreenParametersNotification, object: nil)

        link = view.displayLink(target: self, selector: #selector(tick(_:)))
        link?.add(to: .main, forMode: .common)
    }

    @objc func layout() {
        let screen = NSScreen.screens[0]
        window.setFrame(screen.frame, display: true)
        sim.W = screen.frame.width
        sim.H = screen.frame.height
        CATransaction.begin()
        CATransaction.setDisableActions(true)
        root.bounds = CGRect(origin: .zero, size: screen.frame.size)
        root.position = CGPoint(x: sim.W / 2, y: sim.H / 2)
        root.setAffineTransform(CGAffineTransform(scaleX: 1, y: -1))
        for l in flakeLayers + [pileLayer] {
            l.frame = CGRect(origin: .zero, size: screen.frame.size)
            l.contentsScale = screen.backingScaleFactor
        }
        CATransaction.commit()
    }

    @objc func tick(_ dl: CADisplayLink) {
        let now = dl.targetTimestamp
        let dt = CGFloat(min(0.05, max(0.001, lastTime == 0 ? 1.0 / 60 : now - lastTime)))
        lastTime = now

        let m = NSEvent.mouseLocation
        let c = CGPoint(x: m.x, y: sim.H - m.y)
        var v = CGPoint(x: (c.x - lastMouse.x) / dt, y: (c.y - lastMouse.y) / dt)
        v.x = max(-1500, min(1500, v.x)); v.y = max(-1500, min(1500, v.y))
        mouseVel.x += (v.x - mouseVel.x) * 0.4
        mouseVel.y += (v.y - mouseVel.y) * 0.4
        lastMouse = c

        dockAcc += dt
        if dockAcc > 2 { dockAcc = 0; dockRect = findDock() }
        pollAcc += dt
        if pollAcc >= 0.05 {
            sim.refreshWindows(dock: dockRect, pollDt: pollAcc)
            pollAcc = 0
        }
        sim.step(dt: dt, c: c, cv: mouseVel)

        CATransaction.begin()
        CATransaction.setDisableActions(true)
        let fp = sim.flakePaths()
        for i in 0..<3 { flakeLayers[i].path = fp[i] }
        pileLayer.path = sim.pilePath()
        CATransaction.commit()
    }

    /// Прямоугольник Dock через Accessibility (если доступ выдан и Dock снизу).
    func findDock() -> CGRect? {
        guard AXIsProcessTrusted(),
              let dock = NSRunningApplication.runningApplications(withBundleIdentifier: "com.apple.dock").first
        else { return nil }
        let app = AXUIElementCreateApplication(dock.processIdentifier)
        var kids: CFTypeRef?, pos: CFTypeRef?, size: CFTypeRef?
        guard AXUIElementCopyAttributeValue(app, kAXChildrenAttribute as CFString, &kids) == .success,
              let list = (kids as? [AXUIElement])?.first,
              AXUIElementCopyAttributeValue(list, kAXPositionAttribute as CFString, &pos) == .success,
              AXUIElementCopyAttributeValue(list, kAXSizeAttribute as CFString, &size) == .success,
              let pv = pos, let sv = size,
              CFGetTypeID(pv) == AXValueGetTypeID(), CFGetTypeID(sv) == AXValueGetTypeID()
        else { return nil }
        var p = CGPoint.zero, s = CGSize.zero
        AXValueGetValue(pv as! AXValue, .cgPoint, &p)
        AXValueGetValue(sv as! AXValue, .cgSize, &s)
        let r = CGRect(origin: p, size: s)
        guard r.width > r.height, r.minY < sim.H - 10, r.maxY > sim.H - 150 else { return nil }
        return r
    }

    // MARK: меню

    func menuNeedsUpdate(_ menu: NSMenu) {
        menu.removeAllItems()
        let on = menu.addItem(withTitle: "Снег идёт", action: #selector(toggle), keyEquivalent: "")
        on.state = running ? .on : .off
        menu.addItem(.separator())
        for (i, r) in rates.enumerated() {
            let it = menu.addItem(withTitle: r.0, action: #selector(setRate(_:)), keyEquivalent: "")
            it.tag = i
            it.state = sim.rate == r.1 ? .on : .off
        }
        menu.addItem(.separator())
        menu.addItem(withTitle: "Стряхнуть весь снег", action: #selector(shake), keyEquivalent: "")
        let dock = menu.addItem(withTitle: "Снег на Dock (нужен Универсальный доступ)",
                                action: #selector(askDock), keyEquivalent: "")
        dock.state = AXIsProcessTrusted() ? .on : .off
        menu.addItem(.separator())
        menu.addItem(withTitle: "Выйти", action: #selector(NSApplication.terminate(_:)), keyEquivalent: "q")
    }

    @objc func toggle() {
        running.toggle()
        link?.isPaused = !running
        if running { lastTime = 0; window.orderFrontRegardless() } else { window.orderOut(nil) }
    }

    @objc func setRate(_ sender: NSMenuItem) {
        sim.rate = rates[sender.tag].1
        UserDefaults.standard.set(Double(sim.rate), forKey: "rate")
    }

    @objc func shake() { sim.dumpAll() }

    @objc func askDock() {
        let key = kAXTrustedCheckOptionPrompt.takeUnretainedValue() as String
        _ = AXIsProcessTrustedWithOptions([key: true] as CFDictionary)
    }
}

let app = NSApplication.shared
let delegate = AppDelegate()
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()
