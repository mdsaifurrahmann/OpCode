import Foundation

/// One row of the Stack panel: a word read back from memory near SP.
/// Pure Swift/UI-support data, not part of the FFI surface - derived
/// on-demand from `Emulator.readMemory`.
struct StackEntry: Identifiable, Equatable {
    var id: UInt32 { address }
    let address: UInt32
    let value: UInt16
}

/// The only file that touches the raw generated uniffi types directly -
/// everything else in the app works with plain Swift state published
/// here. Keeping the generated-binding surface contained to one file
/// limits blast radius if that generated API shape changes later.
///
/// Running/stepping is driven from a detached `Task` rather than the
/// synchronous main-actor loop Phase 4 shipped with: a debugger needs to
/// be able to *pause* a long-running program and let the UI keep
/// responding while it runs, and a single blocking `while true { step() }`
/// on the main actor can do neither. `Emulator`'s Rust side is already
/// `Mutex`-guarded (see `x8086-ffi`), so it's safe to call from off the
/// main actor; the detached task calls it in small chunks, hopping back
/// to the main actor between chunks to publish a snapshot and check
/// whether the task was cancelled (that's how Pause works - see `pause()`).
@MainActor
final class EmulatorController: ObservableObject {
    private let emulator: Emulator

    enum ExecutionState: Equatable {
        /// Nothing successfully assembled yet (including "assembled, but
        /// with errors" - there's nothing runnable to step through).
        case notLoaded
        /// A chunked run loop is actively executing on a background task.
        case running
        /// Loaded and not running: freshly assembled, mid-step, paused,
        /// or stopped at a breakpoint. Every stepping/running action is
        /// legal from here.
        case stopped
        case halted
        case waitingForKeyboard
    }

    @Published private(set) var consoleOutput: String = ""
    @Published private(set) var registers: Registers
    @Published private(set) var diagnostics: [Diagnostic] = []
    @Published private(set) var lineToAddress: [LineAddress] = []
    @Published private(set) var executionState: ExecutionState = .notLoaded
    @Published private(set) var canStepBack: Bool = false
    @Published private(set) var watches: [WatchValue] = []
    @Published private(set) var variables: [VariableValue] = []
    @Published private(set) var disassembly: [DisassembledLine] = []
    @Published private(set) var stack: [StackEntry] = []
    @Published var watchError: String?
    /// 0 (slowest, one step visibly at a time) ... 1 (fastest, effectively
    /// instant). Read live by the run loop on every chunk, so dragging
    /// the slider mid-run takes effect immediately.
    @Published var executionSpeed: Double = 1.0

    private var runTask: Task<Void, Never>?

    private let disassemblyWindowSize: UInt32 = 40
    private let stackWindowSize: UInt32 = 16

    var isHalted: Bool { executionState == .halted }
    var isWaitingForKeyboard: Bool { executionState == .waitingForKeyboard }
    var isRunning: Bool { executionState == .running }
    /// Step Into/Over/Run-to-cursor are only meaningful while paused
    /// mid-program - not before anything is loaded, not while a run is
    /// already in flight, and not once halted (there is nothing left to
    /// step forward into). Step *Back* is independent of this - see
    /// `canStepBack`, which stays true even after halting.
    var canStep: Bool { executionState == .stopped }
    var canRun: Bool { executionState != .running }

    /// The source line whose address matches the current IP, if any -
    /// every instruction-producing line has an entry in `lineToAddress`,
    /// so this is `nil` only before anything has been assembled.
    var currentExecutionLine: Int? {
        lineToAddress.first(where: { $0.address == UInt32(registers.ip) }).map { Int($0.line) }
    }

    init() {
        let emulator = Emulator()
        self.emulator = emulator
        self.registers = emulator.registers()
    }

    // MARK: - Run / Pause / Restart / Step

    /// Starts a fresh debug session (assemble, load, sync breakpoints,
    /// run to the first stop) or, if one is already loaded and paused,
    /// simply continues it - the same "Run" gesture serves both roles in
    /// every mainstream debugger, emu8086 included.
    func run(source: String, breakpointLines: Set<Int>) {
        if executionState == .stopped {
            startLoop { [emulator] chunkSize in emulator.run(maxSteps: chunkSize) }
            return
        }
        assembleAndLoad(source: source, breakpointLines: breakpointLines)
        guard executionState == .stopped else { return } // blocked by diagnostics
        startLoop { [emulator] chunkSize in emulator.run(maxSteps: chunkSize) }
    }

    /// Reassembles and reloads `source`, stopping at the entry point
    /// without running - matching emu8086's Restart, which reloads a
    /// fresh debug session but leaves the user in control of when to run.
    func restart(source: String, breakpointLines: Set<Int>) {
        assembleAndLoad(source: source, breakpointLines: breakpointLines)
    }

    func pause() {
        runTask?.cancel()
    }

    /// Awaits completion of any in-flight run/step-over/run-to-cursor
    /// loop. Production UI code never needs this - it just observes
    /// `@Published` state as the loop updates it over time - but tests
    /// need a deterministic way to know a background run has finished
    /// before asserting on that state.
    func waitUntilIdle() async {
        await runTask?.value
    }

    func stepInto() {
        guard canStep else { return }
        applyStepOutcome(emulator.step())
        refreshSnapshot()
    }

    /// Steps over a `CALL` (runs to the instruction right after it,
    /// rather than descending into the subroutine); anything else is
    /// identical to Step Into. Routed through the same chunked/pausable
    /// loop as Run, since the callee could itself run indefinitely.
    func stepOver() {
        guard canStep else { return }
        let currentAddress = UInt32(registers.ip)
        guard let instruction = emulator.disassemble(address: currentAddress, count: 1).first,
            instruction.text.hasPrefix("CALL ")
        else {
            stepInto()
            return
        }
        let targetAddress = currentAddress &+ instruction.byteLen
        startLoop { [emulator] chunkSize in
            emulator.runToCursor(address: targetAddress, maxSteps: chunkSize)
        }
    }

    func stepBack() {
        guard canStepBack, executionState != .running else { return }
        _ = emulator.stepBack()
        if executionState == .halted || executionState == .waitingForKeyboard {
            executionState = .stopped
        }
        refreshSnapshot()
    }

    /// Runs to `address`, or a real breakpoint hit first - the
    /// Run-to-cursor command. Callers translate whatever they have (a
    /// source line via `lineToAddress`, or a disassembly row, which
    /// already carries an address) themselves.
    func runToCursor(address: UInt32) {
        guard canStep else { return }
        startLoop { [emulator] chunkSize in emulator.runToCursor(address: address, maxSteps: chunkSize) }
    }

    func sendKey(_ character: Character) {
        guard executionState == .waitingForKeyboard, let ascii = character.asciiValue else { return }
        emulator.feedKey(scancode: 0, ascii: ascii)
        startLoop { [emulator] chunkSize in emulator.run(maxSteps: chunkSize) }
    }

    // MARK: - Watches

    func addWatch(_ expression: String) {
        watchError = emulator.addWatch(expression: expression)
        refreshWatchesAndVariables()
    }

    func removeWatch(at index: Int) {
        emulator.removeWatch(index: UInt32(index))
        refreshWatchesAndVariables()
    }

    // MARK: - Memory / register live edit

    func readMemory(address: UInt32, len: UInt32) -> Data {
        emulator.readMemory(address: address, len: len)
    }

    func writeMemoryByte(address: UInt32, value: UInt8) {
        emulator.writeMemoryByte(address: address, value: value)
        refreshSnapshot()
    }

    func setRegisters(_ newRegisters: Registers) {
        emulator.setRegisters(registers: newRegisters)
        refreshSnapshot()
    }

    /// Tokenizes `source` for syntax highlighting - routed through here
    /// (rather than called directly from a view) to keep every uniffi
    /// call in one place, even for a stateless helper function like this.
    func tokenize(_ source: String) -> [Token] {
        tokenizeSource(source: source)
    }

    // MARK: - Internal

    private func assembleAndLoad(source: String, breakpointLines: Set<Int>) {
        runTask?.cancel()
        consoleOutput = ""
        diagnostics = []

        let result = emulator.assembleAndLoad(source: source)
        diagnostics = result.diagnostics
        lineToAddress = result.lineToAddress
        syncBreakpoints(breakpointLines)
        executionState = diagnostics.contains(where: { $0.isError }) ? .notLoaded : .stopped
        refreshSnapshot()
    }

    private func syncBreakpoints(_ lines: Set<Int>) {
        emulator.clearBreakpoints()
        for line in lines {
            guard let mapped = lineToAddress.first(where: { $0.line == UInt32(line) }) else {
                continue
            }
            _ = emulator.toggleBreakpoint(address: mapped.address)
        }
    }

    private func applyStepOutcome(_ outcome: StepResult) {
        switch outcome {
        case .continued:
            executionState = .stopped
        case .halted:
            executionState = .halted
        case .waitingForKeyboard:
            executionState = .waitingForKeyboard
        case .decodeError(let message):
            diagnostics.append(
                Diagnostic(line: 0, col: 0, isError: true, message: "decode error: \(message)"))
            executionState = .stopped
        }
    }

    /// Chunk size (steps per `emulator.run`/`runToCursor` call) and the
    /// delay between chunks, both derived from `executionSpeed`. Small
    /// chunks and a real delay make execution visibly animated at the
    /// slow end; one big chunk and no delay make it effectively instant
    /// at the fast end.
    private func currentChunkParameters() -> (chunkSize: UInt32, delayNanoseconds: UInt64) {
        let speed = min(max(executionSpeed, 0), 1)
        let chunkSize = UInt32(1 + speed * 49_999)
        let delayNanoseconds = UInt64((1 - speed) * 0.25 * 1_000_000_000)
        return (chunkSize, delayNanoseconds)
    }

    private func startLoop(_ step: @escaping (UInt32) -> RunResult) {
        executionState = .running
        runTask?.cancel()
        runTask = Task.detached(priority: .userInitiated) { [weak self] in
            while true {
                if Task.isCancelled { break }
                guard let parameters = await self?.currentChunkParameters() else { break }
                let outcome = step(parameters.chunkSize)
                let shouldContinue = await self?.handleChunkOutcome(outcome) ?? false
                if !shouldContinue { break }
                if parameters.delayNanoseconds > 0 {
                    try? await Task.sleep(nanoseconds: parameters.delayNanoseconds)
                }
            }
            await self?.finalizeLoopIfStillRunning()
        }
    }

    /// Publishes a snapshot for this chunk and applies its outcome.
    /// Returns whether the loop should keep going - only `stepLimitReached`
    /// (a runaway-program ceiling, not a real stopping condition) does.
    private func handleChunkOutcome(_ outcome: RunResult) -> Bool {
        refreshSnapshot()
        switch outcome {
        case .halted:
            executionState = .halted
            return false
        case .waitingForKeyboard:
            executionState = .waitingForKeyboard
            return false
        case .breakpointHit:
            executionState = .stopped
            return false
        case .decodeError:
            diagnostics.append(Diagnostic(line: 0, col: 0, isError: true, message: "decode error"))
            executionState = .stopped
            return false
        case .stepLimitReached:
            return true
        }
    }

    /// If the task loop exits because it was cancelled (Pause) rather
    /// than because a chunk outcome already set a terminal state,
    /// `executionState` is still `.running` and needs to land somewhere -
    /// `.stopped`, since a paused program is exactly "loaded, not
    /// running, everything is legal from here."
    private func finalizeLoopIfStillRunning() {
        if executionState == .running {
            executionState = .stopped
            refreshSnapshot()
        }
    }

    private func refreshSnapshot() {
        consoleOutput = emulator.consoleOutput()
        registers = emulator.registers()
        canStepBack = emulator.canStepBack()
        refreshWatchesAndVariables()
        disassembly = emulator.disassemble(address: UInt32(registers.ip), count: disassemblyWindowSize)
        stack = (0..<stackWindowSize).map { offset in
            // `&+`: SP arithmetic genuinely wraps at the 16-bit boundary
            // on real 8086 hardware, matching `Memory`'s own addressing.
            let address = registers.sp &+ UInt16(offset * 2)
            let bytes = [UInt8](emulator.readMemory(address: UInt32(address), len: 2))
            let value = UInt16(bytes[0]) | (UInt16(bytes[1]) << 8)
            return StackEntry(address: UInt32(address), value: value)
        }
    }

    private func refreshWatchesAndVariables() {
        watches = emulator.watchValues()
        variables = emulator.variables()
    }
}
