# TUI framework research for the project/worktree explorer

Researched 16 October 2026; updated after the user's framework and naming decisions. Technical claims use primary sources. No implementation, dependency installation, or comparative performance test was performed.

## Current decision: Rust, Ratatui and Crossterm

The user chose **Rust with Ratatui and Crossterm** for **Tree Rail**, executable **`treerail`**. This supersedes the original recommendation of C# / .NET 10 with Terminal.Gui v2. The .NET comparison below remains a record of the alternatives considered; it does not describe the implementation stack.

The decision reflects the user's preference for the Rust TUI ecosystem and native distribution. **No measured Rust-versus-.NET startup, memory, or rendering advantage has been established for this application.** Ecosystem popularity also does not establish a default visual style: Tree Rail's appearance, interaction model, and responsiveness must be designed and validated.

Ratatui's backend guidance recommends Crossterm when Windows compatibility matters; Crossterm supports Windows and Unix terminals. Ratatui provides responsive layouts and an alternate-screen model for full-screen apps. These capabilities fit the worktree/file/diff panes. [Backend comparison](https://ratatui.rs/concepts/backends/comparison/), [Crossterm documentation](https://docs.rs/crossterm/latest/crossterm/), [layouts](https://ratatui.rs/concepts/layout/), [alternate screen](https://ratatui.rs/concepts/backends/alternate-screen/)

Vim navigation, multikey sequences, focus, text-entry modes, dialogs and diff presentation remain application work. Ratatui's component template demonstrates configurable bindings and sequences; its async-event tutorial demonstrates tick/render events independent of keypresses. Git and GitHub process work must be scheduled without blocking input or drawing. [Configurable bindings](https://ratatui.rs/templates/component/config-rs/), [async events](https://ratatui.rs/tutorials/counter-async-app/full-async-events/)

Ratatui's `TestBackend` enables in-memory rendering checks and snapshot comparisons. These checks can verify pane layout, selected rows, status symbols and diff colors; they do not exercise terminal lifecycle or real keyboard delivery. Interaction/state tests and actual Windows Terminal validation are also needed. [Snapshot testing](https://ratatui.rs/recipes/testing/snapshots/), [TestBackend API](https://docs.rs/ratatui/latest/ratatui/backend/struct.TestBackend.html)

## Native packaging and remaining validation

- **Cargo releases:** build optimized native artifacts for each supported OS/architecture, preserving the application's lockfile for reproducible dependency resolution. `cargo build --release --locked --target <triple>` expresses those build choices. [Cargo build options](https://doc.rust-lang.org/cargo/commands/cargo-build.html)
- **Windows:** use the MSVC target, initially `x86_64-pc-windows-msvc` for x64. Rustup defaults to the MSVC ABI on Windows and recommends it for most purposes; compilation requires its linker and libraries. Those are developer/build-runner requirements, not a requirement to install Rust on end-user machines. [Rustup on Windows](https://rust-lang.github.io/rustup/installation/windows.html), [MSVC prerequisites](https://rust-lang.github.io/rustup/installation/windows-msvc.html)
- **Platform dependencies:** a native executable is still built for a particular target and can link to OS/runtime libraries. Do not promise a fully static universal binary or that one Windows/Linux/macOS artifact works everywhere. Inspect and test the actual release dependencies. [Rust linkage reference](https://doc.rust-lang.org/reference/linkage.html)
- **Release verification:** build and run artifacts on their supported OS/architecture. Verify the selected terminal and Git tools separately from the build toolchain. No release artifact has been built or tested during this research.

Once implementation is authorized, validate a small three-pane screen with a multiline commit form, scoped Vim keys, external status refresh, terminal resize and external Copilot launch. Combine rendering snapshots with meaningful state/process tests, then run the actual release binary in Windows Terminal and on Linux/macOS. The application must restore terminal state on normal exit and errors. These are proposed checks, not completed work.

## Rust configuration and input choices

The product spec selects TOML for editable configuration because it supports comments, tables, arrays, and literal strings suitable for Windows paths. This is a Tree Rail design choice; Ratatui's configurable-binding example demonstrates application-owned configuration rather than imposing a format. Serde supplies serialization/deserialization, and `toml_edit` supports comment-preserving edits. JSON remains useful for generated application state and GitHub CLI responses. [TOML specification](https://toml.io/en/v1.0.0), [Serde](https://serde.rs/), [toml_edit](https://docs.rs/toml_edit/latest/toml_edit/), [Ratatui configuration example](https://ratatui.rs/templates/component/config-rs/)

Use platform directory helpers for configuration and state. `ProjectDirs::state_dir()` is optional and is unavailable on Windows/macOS, so an explicit local-data fallback is needed. Tokio's process module supports subprocess-based Git integration while its runtime schedules background work separately from drawing. [ProjectDirs](https://docs.rs/directories/latest/directories/struct.ProjectDirs.html), [Tokio subprocesses](https://docs.rs/tokio/latest/tokio/process/index.html)

Crossterm distinguishes key press, repeat, and release events. Tree Rail must handle these by input context so a release does not execute a command a second time and navigation can repeat intentionally. Real-terminal tests complement Ratatui's rendering tests. [KeyEventKind](https://docs.rs/crossterm/latest/crossterm/event/enum.KeyEventKind.html)

## Historical .NET recommendation: Terminal.Gui v2

The original recommendation favored C# / .NET 10 with Terminal.Gui v2 because of the user's existing .NET expertise and its documented full-screen controls. That assessment established a credible .NET option; it did not find a functional .NET blocker. The user's subsequent Rust decision supersedes it.

### Current version, not old v1 advice

The official releases page lists **v2.5.0**, dated **11 September**, as its latest non-prerelease release at research time. It explicitly warns that planned API breaks are still accepted in minor v2 releases; 2.5.0 includes changes from 2.4.17. Pin the dependency and upgrade intentionally. Older `gui-cs.github.io/Terminal.GuiV2Docs` material that calls v2 still in development is stale relative to the current `tui-cs` repository and releases. Do not choose v1 merely because of that old notice. [Current releases](https://github.com/tui-cs/Terminal.Gui/releases), [old documentation notice](https://gui-cs.github.io/Terminal.GuiV2Docs/)

The **tagged v2.5.0 project**, rather than just development-branch marketing, targets `net10.0`, C# 14, and declares `IsAotCompatible` and `IsTrimmable`. It also suppresses `IL2026` and `IL3050`, with a comment about source-generated JSON converters. Those flags and comments express intent, not proof that every application and dependency combination publishes and runs correctly. [Tagged project file](https://raw.githubusercontent.com/tui-cs/Terminal.Gui/v2.5.0/Terminal.Gui/Terminal.Gui.csproj)

### Our actual screens

The built-in catalog includes `TreeView<T>`, `ListView`, `TableView`, dialogs, a single-line `TextField`, and a multiline `TextView`. These cover project/worktree navigation, changed-file selection, worktree creation forms, and commit subject/body editing. The diff view still needs application rendering for added/deleted/context lines; the framework does not implement Git semantics for us. [Views catalog](https://tui-cs.github.io/Terminal.Gui/docs/views.html)

The responsive layout and scrollable views support adjacent worktree/file/diff panes. Collapsing panes on narrow terminals remains our product decision, as does maintaining selection across refreshes. The current architecture separates drawing, input, layout, focus, and application instances. [v2 architecture and layout changes](https://tui-cs.github.io/Terminal.Gui/docs/newinv2.html)

### Vim navigation

Commands and bindings exist at view and application scope, with separate focused and hotkey bindings. This gives us the hooks for `j/k`, tree `h/l`, pane focus, `/` search, `gg/G`, and context-specific Git commands. Multi-key commands and any navigation/text-entry modes are application design work, not a promise of a built-in Vim editor. In particular, letters typed into a branch-name or commit field must reach that field rather than activate global shortcuts. [Keyboard handling](https://tui-cs.github.io/Terminal.Gui/docs/keyboard.html)

### Background refresh and Git processes

Terminal.Gui uses a UI main thread. Background status queries, fetches, and process execution should run away from that thread; completed results must be marshalled with the documented application dispatch APIs. Timers are available for refresh. This is a conventional .NET UI model and a good fit for cancellable tasks and a repository service that publishes snapshots. [Background operations](https://tui-cs.github.io/Terminal.Gui/docs/multitasking.html)

### Windows and testing

The driver documentation covers native Windows, Unix/Linux/macOS, a cross-platform `System.Console` driver, and an ANSI driver. The ANSI output implementation documents buffer capture, controlled dimensions, and operation without a console for CI. [Driver API](https://tui-cs.github.io/Terminal.Gui/api/Terminal.Gui.Drivers.html)

Input injection supports keyboard/mouse simulation, deterministic virtual time, and direct or full ANSI pipeline modes. That can verify keyboard-driven workflows and refresh behavior without manually opening a terminal. Real Windows Terminal testing is still necessary for resize, focus, clipboard, and intercepted key combinations. [Input injection](https://tui-cs.github.io/Terminal.Gui/docs/input-injection.html)

The v2 documentation states AOT and single-file support and points to an AOT smoke validation app. That is positive evidence, but this research did not run that app or our eventual binary. The tagged project's suppressed warnings make an actual publish-and-run check more useful than declaring blanket compatibility from the property alone. [AOT documentation](https://tui-cs.github.io/Terminal.Gui/docs/newinv2.html), [tagged build settings](https://raw.githubusercontent.com/tui-cs/Terminal.Gui/v2.5.0/Terminal.Gui/Terminal.Gui.csproj)

## Historical comparison of other .NET choices

| Framework | Evidence relevant to this project | Assessment |
| --- | --- | --- |
| **Hex1b** | Declarative widget tree, resizable splitters, tree/list/table controls, multiline text input, documented Vim remapping, and first-class headless automation. | Strong alternative if declarative UI and automation are the deciding preferences; more moving parts than this external-terminal launcher needs. |
| **XenoAtom.Terminal.UI** | Reactive retained UI with .NET 10/C# 14, splitters, trees, text areas, dialogs, and context commands including multi-stroke sequences. | Credible additional candidate; insufficiently investigated here to displace Terminal.Gui. |
| **Consolonia** | Avalonia-based XAML, bindings, styles, controls and cross-platform terminal rendering. | Attractive when Avalonia/MVVM reuse is a goal; that extra UI stack is not needed for this utility. |
| **SharpConsoleUI** | Full control/layout system, asynchronous per-window model, and documented AOT smoke CI. | Credible modern option, particularly for terminal desktop/window interfaces; extra window/compositor architecture is not a requirement here. |
| **Spectre.Console** | Excellent formatted output, layouts, tables, prompts, and live displays, but interactive displays and prompts cannot be composed concurrently through its supported APIs. | Useful for conventional CLI output; not the recommended main application framework. |

The table is a fit judgment. Source details and limits follow.

### Hex1b

The official release page lists **0.175.0**, dated **15 October**, at research time. The home page's installation example still shows 0.1.0, illustrating why the releases page is more useful for current versions. The getting-started guide asks for .NET 10 or later and ANSI terminal support. [Releases](https://github.com/mitchdenny/hex1b/releases), [getting started](https://hex1b.dev/guide/getting-started)

Its widget catalog includes a tree with multiselect, list/table controls, splitters, responsive layouts, and single/multiline text boxes. Its input guide contains explicit global/scoped Vim remapping examples for lists, tables and trees, making this an unusually close functional match. [Widget catalog](https://hex1b.dev/guide/widgets/), [input handling](https://hex1b.dev/guide/input)

Headless terminals, input sequences, text assertions, and an imperative automator with diagnostic snapshots are documented. The button guide distinguishes a quick awaited handler from long background operations, recommending background state updates plus invalidation to keep rendering responsive. [Testing](https://hex1b.dev/guide/testing), [background work pattern](https://hex1b.dev/guide/widgets/button)

Current source declares AOT compatibility, but also packages native Unix interop libraries and a Windows PTY helper. The source currently targets `net8.0`, while contributor/getting-started material uses a newer SDK; do not mistake the guide's SDK requirement for the library's exact target framework. Helper/native assets mean a truly single-file consumer needs packaging verification. The Windows/Linux/macOS keybinding portability guide is also useful regardless of the chosen framework. [Project source](https://raw.githubusercontent.com/mitchdenny/hex1b/main/src/Hex1b/Hex1b.csproj), [terminal portability](https://hex1b.dev/guide/keybinding-portability)

Our version one launches Copilot in a separate terminal, so an embedded emulator, PTY automation stack, or MCP server is not a requirement. Hex1b's terminal stack may become compelling if the product later hosts agents within its own panes. This is an inference about product fit, not a negative compatibility finding.

### XenoAtom.Terminal.UI

The official repository documents full-screen hosting, rich editing controls, a binding model, focus navigation, splitters, a tree, and commands with multikey sequences. It targets .NET 10/C# 14 and describes its design as NativeAOT-oriented. The releases page lists **3.10.0**, dated **22 September**, including tree-selection and modal-dialog fixes. This was a secondary screening, not an audit of its Native AOT CI, headless test coverage, or supported OS matrix. [Repository](https://github.com/XenoAtom/XenoAtom.Terminal.UI), [releases](https://github.com/XenoAtom/XenoAtom.Terminal.UI/releases)

### Consolonia

The maintainers describe an Avalonia-based cross-platform TUI with familiar bindings, templates and layouts. The README still contains beta-era wording, but the releases page now lists **12.0.3.14**, dated **1 October**, as latest and includes Native AOT hardening. Avoid assessing current release readiness solely from its older introduction. [Repository](https://github.com/Consolonia/Consolonia), [releases](https://github.com/Consolonia/Consolonia/releases)

The merged hardening PR reports an executed `win-x64` Native AOT smoke app, while explicitly retaining upstream warning caveats and reflection bindings in the Gallery example. This is support evidence, not a blanket warning-free guarantee. Its wiki documents a terminal-specific unit-testing facility. [AOT hardening PR](https://github.com/Consolonia/Consolonia/pull/688), [unit testing](https://github.com/Consolonia/Consolonia/wiki/Unit-Testing)

### SharpConsoleUI

The official repository documents Windows, Linux and macOS, .NET 8+, flexible grids, splitters, tree/list controls and multiline editing. Its architecture includes a compositor and independently asynchronous windows. The releases page lists **2.6.5**, dated **4 September**, at research time. [Repository](https://github.com/nickprotop/ConsoleEx), [releases](https://github.com/nickprotop/ConsoleEx/releases)

Its AOT page is particularly concrete: an analyzer-clean library and a publish-and-run headless Linux AOT smoke fixture. It documents an `HtmlControl` dependency caveat requiring scoped warning treatment. That evidence does not by itself prove a Windows/macOS AOT release of our app; the published executable would still need to run there. [AOT validation and caveat](https://github.com/nickprotop/ConsoleEx/blob/master/docs/AOT.md)

### Spectre.Console

Spectre's own documentation says live displays cannot be combined with prompts, progress or status displays, and that those APIs are not thread-safe. A custom low-level event loop could work around this, but would leave us building focus, input routing and forms that dedicated TUI frameworks already provide. This is why attractive screenshots or its layout API are not enough to choose it for the explorer. [Live-display restrictions](https://spectreconsole.net/console/live/live-display/), [prompt restrictions](https://spectreconsole.net/console/how-to/prompting-for-user-input/)

## Historical .NET packaging findings

The original .NET recommendation proposed a self-contained executable, which bundles the runtime without requiring a separate .NET installation. Single-file packaging is distinct from Native AOT; AOT compiles native code without JIT, but imposes trimming and dynamic-code restrictions. These findings explain why .NET remained viable and are no longer the release plan for Tree Rail. [Deployment modes](https://learn.microsoft.com/en-us/dotnet/core/deploying/), [single-file deployment](https://learn.microsoft.com/en-us/dotnet/core/deploying/single-file/overview), [Native AOT](https://learn.microsoft.com/en-us/dotnet/core/deploying/native-aot/)

## Chosen name

**Tree Rail** is the chosen project name; **`treerail`** is the executable name. Earlier alternatives were Treehelm, Workroost, Branchstead and Treeward. The earlier exact-name search was a light collision check, not a trademark, domain, package or executable-name availability guarantee.

Avoid **Grove** for this project: existing projects already use that name for closely related worktree and agent workflows, including [sQVe/grove](https://github.com/sQVe/grove) and [ZiiMs/Grove](https://github.com/ZiiMs/Grove).
