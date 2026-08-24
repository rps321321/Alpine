# Alpine Desktop Apple HIG applicability audit

Audit date: 2026-08-24

Target: Windows-first Tauri desktop application

Baseline: Apple Human Interface Guidelines, applied where the current Alpine
workflow contains the corresponding component or behavior.

## Result

Alpine applies the supplied HIG pages as an interaction and presentation
contract, translated to Windows keyboard conventions where platform commands
differ. The audit does not add controls merely to demonstrate every HIG
component. A guideline is marked not applicable when Alpine has no such input or
content type; adding an unused rating indicator, color well, or image well would
increase complexity without helping the model workflow.

## Component verification

| HIG area | Applicability and Alpine evidence |
| --- | --- |
| Design principles — Delight | Applied through defining moments: immediate hardware discovery, exact model-fit language, stable streaming, recoverable failure, and measured results. Decorative motion is not used as a substitute for feedback. |
| Context menus | Applied to the composer and Task filter menus: short, relevant, keyboard-dismissable lists with visible-interface equivalents. Unsupported attachments remain visibly disabled with a reason. No nested submenu is used. |
| Toolbars | Applied to the top toolbar and Browser chrome: leading navigation/title, a small set of contextual actions, one clear primary action per workflow, and less-frequent tools in the Context Panel. The Browser now exposes Back, Forward, Reload, and address entry together. |
| Progress indicators | Applied. Model downloads show accurate determinate bytes and safe cancellation. Quick and full analysis are mutually exclusive and use a persistent indeterminate progress region because the runtime does not report a reliable total. Full-evaluation phase text replaces generic waiting when emitted. |
| Gauges | Not currently applicable. Alpine has hardware facts and measured values, but no trustworthy bounded target range; inventing a gauge would imply an unsupported optimum. |
| Rating indicators | Not applicable; Alpine does not collect ratings. |
| Keyboards | Applied. All core controls use native keyboard behavior and visible focus. Enter submits, Shift-Enter inserts a line break, Escape dismisses menus/forms, Control-B toggles Projects, Control-Shift-B toggles Browser, and Control/Command-comma opens Settings. Pointer-only alternatives are not required. |
| Pointing devices | Applied through consistent hover/focus behavior, native controls, pointer-captured split resizing, and appropriate resize cursors. Split resizing has equivalent arrow-key operation. |
| Segmented controls | Not used for unrelated actions. Settings sections remain navigation, and the two analysis cards remain independent actions rather than being styled as mutually exclusive segments. |
| Toggles | Applied only to opposing settings states. Text labels and control position expose state without relying on coral/green color alone. |
| Sliders | Not currently applicable. Exact model settings use selects or fields until a bounded continuous value with useful live feedback is available. |
| Color wells | Not applicable; Alpine has no user color-editing workflow. |
| Combo boxes | The installed-model and profile choices use labeled, predictable lists with meaningful defaults. Hugging Face discovery remains search rather than an unbounded combo-box list. |
| Image wells | Not applicable while the initial text-only model descriptor cannot consume images. The composer explains that limitation instead of presenting a false image target. |
| Pickers | Applied to medium-length predictable choices such as model, profile, evaluation scope, and artifact. Search is used for the unbounded remote model catalog. |
| Presentation | Applied through one stable workspace shell, contextual overlays at compact widths, focus-preserving inline feedback, and no unrelated modal succession. |
| Layout and organization | Applied through the resizable three-surface split, stable bottom composer, independently collapsible rails, compact-width overlays, and bounded long-content behavior. |
| Web views | Applied through a contextual Browser with tabs, navigation, address entry, per-host consent, popup interception, isolated application data, and clear-data controls. It is a human surface and does not silently expand agent authority. |
| Settings | Applied. Safe defaults and automatic hardware discovery minimize setup. Infrequent preferences live in an integrated Settings view; task-specific model selection and analysis stay in their workflows. The standard settings shortcut is supported. |
| Materials | Applied sparingly: warm cream/blush layers distinguish rail, canvas, and inspector; shadow and translucency do not obscure hierarchy or text. Reduced motion is honored. |
| Typography | Applied through a restrained Windows system stack, clear hierarchy, bounded line lengths, relative sizing, wrapping for long evidence, and no text embedded in product imagery. |
| Icons | Applied with one consistent Phosphor family. Icon-only buttons have accessible names; status never depends on an icon alone. The Alpine mountain is decorative and has empty alternative text. |
| Color | Applied through the operator-selected coral/orange/rose palette, deep plum text, semantic non-color labels, and automated WCAG AA contrast coverage. |
| Writing | Applied with concise action labels, evidence qualifiers, and actionable errors. Connection failures now identify the local-model problem and offer Retry and Settings recovery. |

## Verification contract

- Focused component tests cover Browser Forward, settings shortcut, mutually
  exclusive analysis progress, actionable task recovery, split-view keyboard
  resizing, narrow-pane defaults, and structural WCAG A/AA rules.
- Visual QA covers wide and compact Home, Model Library, Browser, Settings,
  Analysis and failed Task states in the selected in-app browser.
- The final comparison combines the supplied visual references and a current
  implementation capture in one input before the visual result is accepted.
- The repository canonical verifier remains the release gate after all local
  checks.

## Sources

- [Apple Human Interface Guidelines](https://developer.apple.com/design/human-interface-guidelines/)
- [Design principles — Delight](https://developer.apple.com/design/human-interface-guidelines/design-principles#Delight)
- [Context menus](https://developer.apple.com/design/human-interface-guidelines/context-menus)
- [Toolbars](https://developer.apple.com/design/human-interface-guidelines/toolbars)
- [Progress indicators](https://developer.apple.com/design/human-interface-guidelines/progress-indicators)
- [Web views](https://developer.apple.com/design/human-interface-guidelines/web-views)
- [Settings](https://developer.apple.com/design/human-interface-guidelines/settings)
- [Color](https://developer.apple.com/design/human-interface-guidelines/color)
- [Typography](https://developer.apple.com/design/human-interface-guidelines/typography)
- [Writing](https://developer.apple.com/design/human-interface-guidelines/writing)

The remaining supplied component pages are classified in the table above even
when the current Alpine workflow has no corresponding component.
