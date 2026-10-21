# VCWL
## Vibe Coded Web Language
### Technical Specification & Development Charter

**Project type:** Programming language, compiler, web UI framework and WebAssembly runtime.

**Working name:** VCWL.

**Tagline:** *The web, without the JavaScript.*

**Primary objective:** Create a statically typed, memory-safe-by-default programming language specifically designed for web application development, compiling application code to WebAssembly and providing first-class support for UI components, styling, reactive state, browser APIs and secure application development.

**Development philosophy:** Build a functional, testable implementation before pursuing ambitious optimizations. Prefer working software, reproducible benchmarks and explicit technical decisions over speculative architecture.

**Target platform:** Modern browsers, with static hosting compatibility, including GitHub Pages.

---

# 1. Project Vision

VCWL is a new programming language designed to unify the development of web application logic, user interfaces, styling, event handling and browser integration.

Modern web development frequently combines several distinct technologies:

- HTML for document structure.
- CSS for styling and responsive layouts.
- JavaScript or TypeScript for application logic.
- UI frameworks for component management and reactivity.
- Build tools for bundling, transpilation and optimization.
- Additional libraries for validation, routing, state management and browser API integration.

VCWL aims to provide a coherent programming model that combines these concerns into one language and toolchain.

Developers should be able to write a complete interactive web application in VCWL, compile it to WebAssembly, and deploy the resulting artifacts to a static hosting provider without writing application code in JavaScript.

The language must prioritize:

1. Static type safety.
2. Secure defaults.
3. Predictable application behavior.
4. Efficient DOM updates.
5. Declarative UI development.
6. Integrated styling and responsive layouts.
7. Minimal runtime overhead.
8. Excellent developer ergonomics.
9. Straightforward deployment.
10. Reproducible performance measurements.

VCWL must not merely be a syntax wrapper around JavaScript.

Its compiler, type system, component model, reactive runtime and security architecture must provide meaningful technical benefits.

## 1.1 Fundamental constraint

WebAssembly is the compilation target, not a replacement for the browser host environment.

A standard browser does not expose its DOM, navigation APIs or other browser capabilities directly to arbitrary WASM modules.

VCWL must therefore provide a carefully designed browser integration layer.

The preferred architecture is:

- Application logic compiled to WebAssembly.
- A minimal browser host responsible for accessing browser APIs.
- Compiler-generated bindings or a small, explicitly maintained JavaScript bridge where required.
- No requirement for VCWL application developers to write JavaScript.
- No requirement for developers to write separate CSS or HTML templates for ordinary applications.

The browser host is permitted to use JavaScript internally. The restriction applies to application development, not to the browser's existing implementation or the necessary interoperability layer.

Do not claim that VCWL eliminates JavaScript from the browser itself.

---

# 2. Scope and Non-Goals

## 2.1 Initial scope

The first usable version must support:

- A working compiler.
- A documented source syntax.
- Static type checking.
- WebAssembly code generation.
- A minimal browser runtime.
- Declarative UI components.
- Reactive state.
- Event handling.
- Integrated styling.
- Responsive layouts.
- Safe DOM operations.
- Basic navigation.
- A development server.
- A production build command.
- Automated tests.
- A complete example application.
- Static deployment.

The implementation must be executable, not a collection of mock functions and architectural placeholders.

## 2.2 Non-goals for the initial release

Do not attempt to implement all of the following before the first usable release:

- A complete replacement for every browser API.
- A replacement for every CSS feature.
- A custom browser engine.
- A complete custom text-rendering engine.
- A new HTTP protocol.
- A custom operating system.
- A package registry.
- A distributed build system.
- A complete IDE.
- A replacement for JavaScript inside browsers.
- A guarantee of immunity from all security vulnerabilities.
- A universal performance advantage over every JavaScript framework.

These features may be investigated later if the architecture justifies them.

## 2.3 Definition of success

VCWL succeeds when a developer can create, type-check, compile, test and deploy a responsive interactive website without authoring JavaScript, CSS or HTML application code.

The generated application must run in an ordinary supported browser.

A minimal generated HTML entry point and browser integration layer are acceptable.

---

# 3. Language Design

## 3.1 General characteristics

VCWL must be:

- Statically typed.
- Compiled.
- Expression-oriented where practical.
- Declarative for UI construction.
- Imperative where appropriate for application logic.
- Memory-safe by default.
- Explicit about fallible operations.
- Designed around predictable reactive updates.
- Capable of compiling to WebAssembly.
- Suitable for large applications as well as small websites.

The language should favor readability and explicit semantics over clever syntax.

Avoid introducing unnecessary syntax merely to make the language look unique.

The initial grammar must be documented and implemented by a real parser.

## 3.2 Illustrative syntax

The following example illustrates the intended developer experience. It is a design proposal, not an assertion that the syntax already exists.

```vcwl
app "Hello VCWL"

component App {
    state clicks: Int = 0

    style {
        background: #0b0d10
        color: #f5f5f5
        font: system-ui
        padding: 2rem
    }

    column(gap: 20px, align: center) {
        text("Hello, WebAssembly!")

        text("Clicked {clicks} times")

        button {
            text("Click me")

            on click {
                clicks += 1
            }

            style {
                background: #b6ff6c
                color: #111111
                radius: 8px
                padding: 12px 20px
            }
        }
    }
}
```

The first implementation does not need to support this entire example immediately.

However, the final syntax should be coherent with it, and the initial subset must have an explicit specification.

The compiler must reject unsupported syntax with actionable errors rather than silently ignoring it.

## 3.3 Required language constructs

Implement the following in a deliberate order.

### Phase A: Core language

- Module declarations.
- Variables and immutable bindings.
- Mutable bindings.
- Primitive types.
- Functions.
- Function parameters and return types.
- Conditional expressions.
- Loops.
- Structs or records.
- Enumerations.
- Arrays and collections.
- String interpolation.
- Comments.
- Imports and exports.
- Compile-time diagnostics.

### Phase B: Type system

- Static type checking.
- Type inference for straightforward expressions.
- Explicit type annotations.
- Generic functions and types.
- Algebraic data types.
- Pattern matching.
- Optional types.
- Result types for fallible operations.
- Exhaustive pattern checking.
- Explicit conversions where appropriate.

### Phase C: Web language

- Components.
- Properties and typed component parameters.
- Reactive state.
- Event handlers.
- Styling declarations.
- Layout primitives.
- Conditional rendering.
- Lists and keyed iteration.
- Component composition.
- Browser capabilities.
- Routing and navigation.
- Typed network requests.

The compiler must distinguish syntax errors, type errors, compilation errors and runtime errors.

---

# 4. Type System and Safety

Type safety is a core feature, not an optional enhancement.

## 4.1 Static typing

The compiler must detect incompatible operations before generating the final application binary.

Examples of invalid operations include:

- Adding a string to an integer without an explicit conversion.
- Assigning an incompatible value to a variable.
- Calling a function with incorrect argument types.
- Accessing a field that does not exist.
- Using an optional value without handling its absence.
- Returning a value that violates the declared return type.
- Supplying a string where a typed CSS length is required.

The compiler must report the source location and explain the error.

## 4.2 Optional values

Use explicit optional types rather than allowing null values to appear implicitly.

Illustrative syntax:

```vcwl
let username: String? = get_username()

match username {
    Some(name) => {
        text(name)
    }

    None => {
        text("Guest")
    }
}
```

The exact syntax may change during implementation, but optional values must have well-defined semantics.

## 4.3 Error handling

Provide a standard result type.

Illustrative syntax:

```vcwl
type Result<T, E> =
    | Ok(T)
    | Err(E)
```

Fallible operations must communicate failure through documented mechanisms.

Do not silently convert failed network requests, invalid input or parsing errors into arbitrary default values.

## 4.4 Runtime data validation

External data must not become trusted merely because the developer supplies a type annotation.

For example, decoding a JSON response into a `User` must validate the actual runtime structure.

The implementation should provide typed decoding with explicit errors.

Validation must cover the supported types, required fields, optional fields, collections and malformed input.

## 4.5 Memory safety

Use WebAssembly's isolated linear memory and a runtime memory model designed to prevent unsafe application-level memory access.

Investigate suitable implementation strategies:

- A managed memory model.
- Ownership and borrowing.
- Reference-counted resources.
- Explicit resource handles.
- A hybrid approach.

Select one primary strategy for the MVP rather than implementing several incomplete alternatives.

Memory safety must be evaluated at the language and runtime levels. WebAssembly isolation alone does not guarantee that application logic is correct or memory-safe.

Any unsafe facilities must be explicit, narrowly scoped and documented.

---

# 5. UI Component System

VCWL must provide a first-class component model.

## 5.1 Components

A component must be able to:

- Accept typed properties.
- Declare local reactive state.
- Render child components.
- Handle events.
- Define styling.
- Render conditionally.
- Iterate over collections.
- Compose with other components.
- Clean up resources when removed.

Components must have deterministic lifecycle semantics.

## 5.2 Built-in UI primitives

The initial standard UI library must include:

- Text.
- Headings.
- Buttons.
- Links.
- Containers.
- Rows.
- Columns.
- Stacks.
- Images.
- Inputs.
- Text areas.
- Checkboxes.
- Select controls.
- Lists.
- Forms.
- Scrollable containers.

Use semantic native HTML elements wherever appropriate.

A button should become an actual accessible HTML button, not a generic `div` pretending to be one.

Components should expose accessible labels, keyboard behavior and other relevant semantics through typed APIs.

## 5.3 Conditional rendering

Illustrative syntax:

```vcwl
if authenticated {
    Dashboard()
} else {
    LoginPage()
}
```

The runtime must mount and unmount the relevant elements correctly.

Removed components must not leave behind active event handlers, subscriptions or timers.

## 5.4 Lists and keyed rendering

Support iteration over collections.

Illustrative syntax:

```vcwl
for user in users key user.id {
    UserCard(user: user)
}
```

The compiler or runtime must use stable keys to preserve element identity when appropriate.

Incorrect or duplicate keys should produce useful diagnostics in development builds.

---

# 6. Reactive Runtime and DOM Optimization

The runtime must minimize unnecessary rendering work.

The initial implementation should use fine-grained reactivity combined with compiler-generated update operations.

## 6.1 Reactive state

Illustrative syntax:

```vcwl
component Counter {
    state count: Int = 0

    text("Count: {count}")

    button {
        text("Increment")

        on click {
            count += 1
        }
    }
}
```

When `count` changes, the runtime should update only the DOM operations that depend on that state.

Unrelated components should not be rerendered without a reason.

## 6.2 Dependency tracking

The compiler should identify dependencies between reactive values and UI expressions wherever practical.

Example:

```text
count
  ├── Counter label
  └── Counter accessibility description
```

Changing `count` should invalidate the relevant expressions without forcing an application-wide rerender.

The implementation must define how it handles:

- Multiple dependent expressions.
- Batched updates.
- Nested components.
- Derived values.
- Conditional dependencies.
- Removed components.
- Circular reactive dependencies.

Circular dependencies must not cause uncontrolled update loops.

## 6.3 DOM bridge

The browser host should expose a narrow set of operations, such as:

- Create an element.
- Create a text node.
- Append a child.
- Remove a node.
- Update text.
- Update an attribute.
- Update a property.
- Register an event handler.
- Remove an event handler.
- Update a style property.
- Read the viewport dimensions.
- Request navigation.

The exact interface must be designed to minimize expensive WASM-to-host calls.

Batch DOM operations where appropriate.

Do not assume that moving application logic to WASM makes every operation faster. Crossing the WASM/browser boundary has a cost, and the runtime must account for it.

## 6.4 No mandatory virtual DOM

A virtual DOM must not be a mandatory part of the architecture.

Prefer compiler-generated DOM updates and fine-grained reactivity.

A reconciliation mechanism may be introduced where it solves a demonstrated problem, but it must be justified by benchmarks.

---

# 7. Integrated Styling System

Styling must be part of VCWL source code.

Developers should not need to author separate CSS files for ordinary applications.

## 7.1 Initial styling features

Implement:

- Colors.
- Backgrounds.
- Width and height.
- Minimum and maximum dimensions.
- Margin and padding.
- Borders.
- Border radii.
- Typography.
- Flexbox-style layouts.
- Grid layouts.
- Gaps.
- Alignment.
- Positioning.
- Overflow.
- Opacity.
- Transforms.
- Transitions.
- Responsive conditions.

The implementation should translate supported styling declarations into optimized native browser styling operations.

Do not build a complete replacement for the browser's CSS layout engine in the MVP.

## 7.2 Typed style values

Define dedicated types for values such as:

- Length.
- Percentage.
- Color.
- Duration.
- Opacity.
- Font weight.
- Display mode.
- Alignment.
- Overflow behavior.

Reject invalid combinations at compile time.

For example, a numeric length must not be accepted where a color is required.

Where CSS permits multiple forms of a value, the language must document which forms are supported.

## 7.3 Responsive design

Responsive rules must be expressible directly in VCWL.

Illustrative syntax:

```vcwl
style Dashboard {
    columns: 3
    gap: 24px

    when viewport < 900px {
        columns: 2
        gap: 16px
    }

    when viewport < 600px {
        columns: 1
        gap: 12px
    }
}
```

The compiler should generate efficient native browser styling rules or viewport-dependent operations.

Prefer CSS media queries for straightforward responsive styling because the browser already optimizes their evaluation.

Support viewport and container-based responsive behavior as the implementation matures.

The layout must remain functional on mobile devices, tablets and desktop displays.

---

# 8. Security Architecture

Security must be built into the language design and runtime rather than added after the initial implementation.

## 8.1 Capability-based access

Applications must not receive unnecessary browser capabilities automatically.

Introduce an explicit capability declaration model.

Illustrative syntax:

```vcwl
app "Dashboard" {
    capabilities {
        network: allow("api.example.com")
        storage: local
    }
}
```

The exact permission syntax is a design decision to validate during implementation.

Potential capabilities include:

- Network access.
- Local storage.
- Session storage.
- Clipboard access.
- Notifications.
- File selection.
- Camera and microphone access.
- Geolocation.
- Downloads.

Capabilities must be enforced by the host runtime, not merely recorded in compiler metadata.

If a module lacks a capability, the host must refuse the corresponding operation.

## 8.2 Browser security

The runtime must:

- Use safe text insertion by default.
- Avoid interpreting untrusted strings as executable HTML.
- Avoid generating inline event-handler attributes.
- Respect browser origin and CORS restrictions.
- Validate external data.
- Restrict application access to explicitly granted capabilities.
- Avoid leaking sensitive data through debug logs.
- Handle permission denial and network failures correctly.

Do not claim that static typing or WASM automatically prevents XSS, CSRF or other web vulnerabilities.

These require specific architectural and runtime protections.

## 8.3 Resource limits

Where practical, support configurable limits on:

- Memory consumption.
- Execution time.
- Event-handler execution.
- Network response sizes.
- Collection sizes.
- Recursive operations.

Browser execution cannot always be interrupted safely at an arbitrary instruction boundary without additional runtime support.

Document which resource limits are enforced and which are best-effort.

## 8.4 Security testing

Add tests for:

- Invalid capability access.
- Malformed network responses.
- Unsafe HTML insertion.
- Invalid type conversions.
- Resource cleanup.
- Unexpected host failures.
- Malformed application binaries.
- Untrusted input handling.

Security tests must verify actual enforcement, not simply confirm that configuration files contain permission declarations.

---

# 9. Browser Integration

The initial target is the browser, not a standalone WASI application.

## 9.1 Browser host

Implement the smallest practical host that provides:

- WASM module loading.
- DOM integration.
- Event dispatch.
- Browser API bindings.
- Navigation.
- Viewport observation.
- Runtime initialization.
- Error reporting.
- Resource cleanup.

The host may be written in JavaScript because browsers expose these APIs through their existing web platform.

However:

- VCWL application developers must not need to write JavaScript.
- Application logic must not silently fall back to JavaScript.
- Host code must remain separate from generated application code.
- The host interface must be documented and tested.
- The host should be kept small and auditable.

## 9.2 Browser compatibility

Target modern versions of major browsers.

Test at minimum:

- Chromium-based browsers.
- Firefox.
- Safari.

Document required WebAssembly features and any browser-specific limitations.

Avoid depending on experimental WebAssembly features unless they are optional enhancements.

## 9.3 Accessibility

Native semantic elements must be preferred when they are suitable.

Support:

- Keyboard navigation.
- Focus management.
- Accessible names.
- Form labels.
- Appropriate roles.
- Visible focus indicators.
- Reduced-motion preferences.
- Screen-reader-compatible content.

Accessibility is a functional requirement, not a cosmetic improvement.

---

# 10. Networking and Data

VCWL must provide typed APIs for network operations.

## 10.1 HTTP client

Provide a standard HTTP client supporting:

- GET.
- POST.
- PUT.
- PATCH.
- DELETE.
- Request headers.
- Request bodies.
- JSON serialization.
- JSON decoding.
- Timeouts where supported.
- Cancellation.
- Explicit error handling.

Use the browser's native networking implementation through the host rather than implementing HTTP directly over raw browser-independent sockets.

## 10.2 Typed requests

Illustrative syntax:

```vcwl
type User = {
    id: Int,
    name: String,
    email: String
}

async fn get_user(id: Int) -> Result<User, HttpError> {
    let response = http.get("/api/users/{id}").await?

    return response.json<User>()
}
```

The actual language may use different async semantics, but network operations must have well-defined error and cancellation behavior.

## 10.3 Security

Never treat compile-time types as substitutes for runtime response validation.

The HTTP client must reject malformed responses and communicate decoding failures explicitly.

Cross-origin restrictions and browser security policies must remain in effect.

---

# 11. Toolchain and Developer Experience

VCWL must provide a coherent command-line interface.

The executable should be named `vcwl`.

## 11.1 Required commands

Implement commands equivalent to:

```bash
vcwl new my-app
vcwl dev
vcwl check
vcwl build
vcwl test
vcwl fmt
vcwl clean
```

Expected behavior:

### `vcwl new`

Creates a working starter application.

### `vcwl dev`

Starts a local development server with useful diagnostics and automatic rebuilds.

Hot reload is desirable, but must not compromise application correctness.

### `vcwl check`

Parses the project, resolves imports, performs type checking and reports diagnostics without requiring a production build.

### `vcwl build`

Compiles the application into deployable static assets.

### `vcwl test`

Runs language tests, runtime tests and application tests.

### `vcwl fmt`

Formats source files deterministically.

### `vcwl clean`

Removes generated build artifacts without deleting source files.

## 11.2 Project structure

A generated project should have a simple, documented structure.

For example:

```text
my-app/
├── vcwl.toml
├── src/
│   ├── main.vcwl
│   └── components/
├── tests/
└── dist/
    ├── index.html
    ├── app.wasm
    └── runtime.js
```

This structure is illustrative.

The implementation may choose a different layout if there is a compelling technical reason.

The production output should include only the artifacts needed to run the application.

## 11.3 Diagnostics

Compiler errors must include:

- File path.
- Line and column.
- Error category.
- Clear explanation.
- A relevant source excerpt where practical.
- A suggested correction where it can be determined reliably.

Do not expose raw compiler crashes as ordinary diagnostics.

## 11.4 Documentation

Maintain:

- Language reference.
- Type-system specification.
- Standard library reference.
- Browser host API documentation.
- Installation instructions.
- Tutorial.
- Example projects.
- Troubleshooting guide.
- Architecture decisions.
- Security model.
- Known limitations.

Documentation must match the implemented behavior.

---

# 12. Compiler Architecture

The compiler must have a maintainable internal architecture.

Suggested stages:

1. Lexical analysis.
2. Parsing.
3. Abstract syntax tree construction.
4. Name resolution.
5. Type checking.
6. Semantic validation.
7. Intermediate representation.
8. Optimization.
9. WebAssembly code generation.
10. Binary validation.
11. Artifact generation.

The compiler must reject invalid programs before producing a deployable build.

## 12.1 Intermediate representation

Introduce an intermediate representation that separates language semantics from WebAssembly code generation.

This allows future targets or optimization passes without rewriting the entire parser and type checker.

## 12.2 WebAssembly backend

Generate valid WebAssembly modules.

Validate the generated modules using an independent WebAssembly implementation or validation tool.

Do not rely exclusively on the compiler's own internal validation.

## 12.3 Optimization

Prioritize correctness before aggressive optimization.

Potential optimizations include:

- Dead-code elimination.
- Constant folding.
- Function inlining where beneficial.
- Removal of unused imports.
- Static UI dependency analysis.
- Elimination of unnecessary reactive subscriptions.
- Batched DOM operations.
- Static style extraction.
- Reduced runtime allocations.
- Binary size optimization.

Every optimization must preserve documented language semantics.

Optimizations should be covered by regression tests.

## 12.4 Compiler implementation language

Select a suitable implementation language for the compiler and justify the decision.

The compiler itself does not have to be written in VCWL.

However, it must compile VCWL application code into WebAssembly, and the application language must not secretly depend on transpiling the user's code into ordinary JavaScript.

A JavaScript host runtime is permitted.

---

# 13. Standard Library

Build a small, coherent standard library.

Initial modules should cover:

- Strings.
- Collections.
- Numeric operations.
- Formatting.
- JSON.
- Errors and results.
- Time and durations.
- URL handling.
- HTTP.
- UI components.
- Styling.
- Events.
- Browser capabilities.

Do not create a massive standard library before the compiler can support the features it needs.

Prefer a small number of well-designed abstractions over numerous inconsistent helpers.

All public APIs must have documented types and error behavior.

---

# 14. Testing and Quality Assurance

Automated testing is mandatory.

## 14.1 Compiler tests

Cover:

- Valid programs.
- Invalid syntax.
- Type errors.
- Generic types.
- Optional values.
- Pattern matching.
- Import resolution.
- Invalid component properties.
- Invalid style values.
- Invalid capability declarations.

## 14.2 Runtime tests

Cover:

- WASM initialization.
- Memory management.
- Reactive state.
- DOM creation and updates.
- Event registration and removal.
- Component mounting and unmounting.
- Async operations.
- Error handling.
- Cleanup.

## 14.3 Browser tests

Use browser automation to verify real behavior.

The test suite must check:

- Application startup.
- Rendering.
- Button interaction.
- Form input.
- Reactive updates.
- Navigation.
- Responsive layouts.
- Keyboard interaction.
- Browser console errors.
- Accessibility basics.
- Capability enforcement.

## 14.4 Regression tests

Every fixed bug must receive a regression test when practical.

The compiler and runtime must not silently accept invalid programs because a feature is partially implemented.

## 14.5 Continuous integration

Set up automated checks for:

- Formatting.
- Linting.
- Compiler tests.
- Runtime tests.
- Browser tests where practical.
- WebAssembly validation.
- Production builds.
- Dependency security checks.

The repository must provide reproducible build instructions.

---

# 15. Performance Requirements

VCWL must be benchmarked against comparable applications written using established web technologies.

Do not claim performance advantages without measurements.

## 15.1 Required metrics

Measure:

- Generated WASM binary size.
- Browser host size.
- Total compressed production size.
- Initial startup time.
- Time to first visible content.
- Initial DOM node count.
- Memory consumption.
- State update latency.
- DOM mutations per interaction.
- CPU time during representative interactions.
- Network request overhead.

## 15.2 Comparison targets

Create equivalent applications using:

- Plain JavaScript with direct DOM updates.
- A modern component-based framework.
- VCWL.

Where appropriate, also compare a fine-grained reactive implementation.

Use equivalent functionality, similar markup, the same browser, identical hardware and repeatable workloads.

Report results honestly.

If VCWL is slower or larger for a particular workload, document the result and investigate why.

## 15.3 Initial performance goals

Treat these as engineering targets, not guaranteed results:

- Avoid unnecessary application-wide rerenders for localized state changes.
- Minimize avoidable WASM-to-host calls.
- Keep the browser runtime small.
- Remove unused code from production builds.
- Avoid excessive allocations during ordinary UI interactions.
- Keep the default starter application small.
- Preserve native browser layout and text-rendering performance.

Set numerical performance budgets only after measuring a working baseline.

---

# 16. Deployment

VCWL applications must support static deployment.

## 16.1 GitHub Pages

The build command must generate all files required for a static deployment.

The application must not require:

- A proprietary server.
- A continuously running backend.
- A paid hosting service.
- Runtime compilation on the hosting server.

The generated site must support GitHub Pages, including deployment under a repository subpath.

Handle asset paths and WebAssembly loading correctly.

Ensure the deployment instructions address MIME types, base paths, browser compatibility and routing behavior.

## 16.2 Other hosting providers

Keep the generated output compatible with conventional static hosts wherever practical.

A future deployment adapter may support additional hosting platforms, but GitHub Pages is the primary acceptance target.

---

# 17. Development Roadmap

Development must proceed through demonstrable milestones.

Do not attempt to implement every feature simultaneously.

## Milestone 0: Repository and architecture

Deliver:

- Repository structure.
- Architecture document.
- Initial language specification.
- Compiler implementation plan.
- Chosen implementation language.
- Dependency assessment.
- Initial test strategy.
- Explicit list of accepted trade-offs.

Acceptance criteria:

- The project builds.
- The architecture is documented.
- The implementation strategy is justified.
- No major component is represented as implemented when it is only planned.

## Milestone 1: First executable program

Deliver a minimal VCWL program that compiles to WebAssembly.

Example:

```vcwl
fn main() -> Int {
    return 42
}
```

Acceptance criteria:

- The source parses.
- The compiler checks its types.
- The compiler generates a valid WASM binary.
- An independent WASM runtime executes the module.
- The exported function returns the expected result.
- Invalid programs produce useful diagnostics.

This milestone must work before beginning the full UI framework.

## Milestone 2: Hello World website

Deliver a complete browser application.

Example:

```vcwl
app "Hello World"

component App {
    text("Hello, WebAssembly!")
}
```

Acceptance criteria:

- The application compiles.
- The production output loads in an ordinary browser.
- The text is rendered through native browser elements.
- No application JavaScript is required.
- The minimal host initializes the WASM application.
- Browser errors are handled and reported clearly.

## Milestone 3: Interactive components

Deliver:

- Buttons.
- Event handlers.
- Typed state.
- Reactive text.
- Component composition.
- Conditional rendering.
- Resource cleanup.

Acceptance criteria:

- Clicking a button updates the relevant text.
- Unrelated DOM nodes are not unnecessarily recreated.
- Removed components release their event handlers and subscriptions.
- Invalid event handlers are rejected where their types are incompatible.

## Milestone 4: Styling and responsive layouts

Deliver:

- Integrated style syntax.
- Typed style values.
- Flexbox-style layouts.
- Grid layouts.
- Responsive breakpoints.
- Native semantic HTML output.

Acceptance criteria:

- A complete responsive application can be written without a separate CSS source file.
- Invalid style values are rejected.
- The application works at mobile, tablet and desktop viewport sizes.
- Native browser layout is used where appropriate.

## Milestone 5: Developer toolchain

Deliver:

- Project generator.
- Development server.
- Production builder.
- Formatter.
- Test runner.
- Documentation.
- Example projects.

Acceptance criteria:

A new developer can create and run a project using documented commands without manually editing generated JavaScript.

## Milestone 6: Security and networking

Deliver:

- Typed HTTP APIs.
- Runtime data validation.
- Explicit capabilities.
- Host-enforced permissions.
- Safe DOM APIs.
- Security tests.

Acceptance criteria:

- Malformed JSON cannot silently become a trusted typed value.
- Unauthorized host operations are rejected.
- Untrusted text is not interpreted as executable HTML by default.
- Network failures are represented explicitly.

## Milestone 7: Optimization and stabilization

Deliver:

- Compiler optimization passes.
- Production binary-size optimization.
- Performance benchmarks.
- Browser compatibility testing.
- Security review.
- Reproducible builds.
- A documented release process.

Acceptance criteria:

- All critical tests pass.
- Generated modules validate independently.
- Performance claims are supported by measurements.
- Known limitations are documented.
- The starter application can be deployed successfully to GitHub Pages.

---

# 18. Repository and Engineering Standards

The repository must remain understandable and maintainable.

## 18.1 Code quality

- Use descriptive names.
- Avoid unnecessary abstraction.
- Separate compiler, runtime and browser host concerns.
- Document non-obvious language semantics.
- Keep functions focused.
- Handle errors explicitly.
- Avoid global mutable state unless justified.
- Keep generated files separate from source files.
- Do not commit generated artifacts unless the repository intentionally tracks them.

## 18.2 Technical decisions

Create architecture decision records for major choices, including:

- Compiler implementation language.
- Type-system design.
- Memory management.
- Intermediate representation.
- WASM code generation.
- Browser host interface.
- Reactive update model.
- Styling representation.
- Package and module resolution.
- Security capability enforcement.

Each record must explain the decision, alternatives, trade-offs and consequences.

## 18.3 Dependency policy

Prefer established, maintained tools for infrastructure that is not the project's main innovation.

Avoid reinventing:

- WebAssembly binary validation.
- Browser automation.
- Cryptographic primitives.
- General-purpose testing infrastructure.

The language, compiler semantics, reactive model and integration architecture are the main areas where custom implementation effort is justified.

Pin dependencies and provide reproducible setup instructions.

## 18.4 No fake completion

A feature is not complete merely because:

- Its API exists.
- Its type definitions compile.
- Its documentation has been written.
- A mock implementation returns plausible results.
- A placeholder passes a superficial test.

A feature is complete when it works in an executable application and has appropriate tests.

---

# 19. Acceptance Criteria for Version 1.0

VCWL 1.0 is considered usable when all the following conditions are met.

### Language

- A documented grammar exists.
- The parser supports the documented syntax.
- The type checker rejects invalid programs.
- Functions, records, collections and error handling work.
- Compiler diagnostics identify source locations.

### Compilation

- Valid source code compiles to WebAssembly.
- Generated binaries validate independently.
- The compiler does not require JavaScript application code generation.
- Production builds are reproducible.

### UI

- Components render as native browser elements.
- Typed properties work.
- Reactive state works.
- Events work.
- Conditional rendering works.
- Removed components clean up resources.
- Integrated styling works.
- Responsive rules work.

### Security

- Untrusted data is validated at runtime.
- Capability restrictions are enforced by the host.
- Safe text insertion is the default.
- Security limitations are documented.

### Tooling

- Project creation works.
- Development server works.
- Type checking works.
- Production builds work.
- Tests run through documented commands.
- Example applications compile successfully.

### Deployment

- The generated output works on GitHub Pages.
- Repository subpaths are supported.
- WebAssembly loads correctly.
- The application works in supported browsers.
- No server-side component is required for a basic static application.

### Documentation

- Installation and usage are documented.
- The language reference matches the implementation.
- Browser integration is explained.
- Deployment instructions work.
- Known limitations are explicit.

---

# 20. Instructions to the Autonomous Implementer

You are responsible for implementing VCWL as a functioning software project.

You may use an AI coding assistant, but the resulting codebase must stand on its own.

## Required behavior

1. Inspect the existing repository before changing anything.
2. Identify existing implementations and preserve useful work.
3. Establish the architecture and write a concise implementation plan.
4. Begin with the smallest executable compiler milestone.
5. Run the tests after each meaningful change.
6. Fix failing tests before introducing unrelated features.
7. Maintain documentation as the language evolves.
8. Record major architectural decisions.
9. Commit changes in logical, reviewable increments if Git access is available.
10. Never claim that a feature works without testing it.
11. Never silently substitute a mock implementation for a real feature.
12. Keep the main branch buildable.
13. Prefer completing a smaller working vertical slice over creating a large unfinished architecture.
14. If a requirement is technically impossible under the current architecture, explain the limitation and propose the smallest reasonable alternative.
15. Continue autonomously through implementation, testing, documentation and stabilization.

## Prioritization

When choosing between competing tasks, prioritize:

1. Correctness.
2. Executability.
3. Type safety.
4. Security.
5. Developer usability.
6. Maintainability.
7. Performance.
8. Convenience features.

Do not sacrifice correctness or security merely to achieve a smaller binary or a more impressive benchmark.

## Decision-making autonomy

You may choose the compiler implementation language, parser technology, WebAssembly generation strategy, memory model and internal architecture.

Document significant decisions and their trade-offs.

Do not ask the project owner to make routine implementation decisions that can be resolved through research, prototyping or testing.

Ask for clarification only when an ambiguity materially affects the project's direction, compatibility or security.

## Final deliverables

Produce:

- A working compiler.
- A documented language specification.
- A functioning reactive UI runtime.
- A browser integration layer.
- A command-line toolchain.
- Automated tests.
- A security model.
- Performance benchmarks.
- Example applications.
- A GitHub Pages deployment example.
- Installation and contribution documentation.
- A clear list of implemented features and remaining limitations.

The ultimate goal is not to produce the largest possible codebase.

It is to produce a coherent, technically defensible programming language that developers can actually use to build web applications.

**Build the language, prove that it works, measure its performance, and let the results determine the next stage of development.**
