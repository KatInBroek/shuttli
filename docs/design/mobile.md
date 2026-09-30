# Mobile product design requirements

## Purpose and design responsibility

This brief is for the designers already familiar with the product's Web design. Design native iPhone and Android phone experiences for using clipboard content across devices. Continue the established brand and logo; the current display name is defined in [branding/name.txt](../../branding/name.txt).

The requirements below define user needs, product behavior and constraints. The design team owns information architecture, navigation, content grouping, layouts, controls, detailed interactions, visual treatment and final microcopy. Choose appropriate solutions for each platform. Existing mobile scaffolds and screen/action examples in engineering specifications are implementation context, not a prescribed design or visual acceptance baseline. Preserve the behavioral requirements when proposing a different experience.

## Product context and first release scope

People use computers continuously but often open the phone app only briefly. A person may copy something on a computer while the phone app is closed, then open the phone app later to find and use it. They may also want to send something copied on their phone to one or more computers.

The first release supports text and supported clipboard images, multiple computers, per-device permissions, recent history, explicit phone sending and explicit copying to the phone clipboard. Describe images as images in user-facing copy; image files copied as files are outside this release.

The app works through the separately installed official Tailscale client and compatible computer applications. It has no additional product account, central clipboard store, QR pairing or address-entry setup. Tailscale has its own account and connection requirements. At least one compatible computer must discover the phone on first use; the phone can then learn about other computers and reconnect to previously authenticated devices. Discovery can take time and can be incomplete.

The phone app performs synchronization work while open in the foreground. It does not monitor the phone clipboard in the background or promise delivery while closed. Later reopening retrieves what permitted computers still retain. File transfer, standalone LAN discovery, push delivery, share extensions, shortcuts and background services are outside this release.

## User needs and scenarios

| Scenario | Required user outcome |
| --- | --- |
| First use | Understand prerequisites, identify the right computer and understand the permission needed there before its history becomes available. |
| Returning after the app was closed | Find recent retained copies from permitted computers, understand how fresh the results are and recognize possible gaps. |
| Several computers | Distinguish sources, locate the desired text or image and understand that each computer has independent availability and permissions. |
| Using a computer copy on the phone | Inspect the selected content and deliberately place that exact content on the phone clipboard for use in another app. |
| Sending phone content | Deliberately import the current clipboard, understand the captured content and eligible destinations, then explicitly authorize sending. |
| Checking a transfer | Determine which destinations succeeded, failed or have an unknown result. |
| Reusing earlier phone content | Explicitly resend a retained phone-origin item using current permissions without replacing it with today's clipboard. |
| Changing access | Understand both directions for each device, the effect of permission changes and what already retrieved content cannot be recalled. |
| Limited or missing data | Understand an offline source, unavailable body, disabled history or retention gap without mistaking it for a successful empty refresh. |
| Clearing history | Understand what is removed locally and why permitted remote copies may return on a later refresh. |

These are user tasks and outcomes. The design team determines the journeys and their presentation.

## Device access and discovery

- Sending from the phone and receiving into the phone app are independent permissions for each device. New devices default to sending off and receiving on. Mobile has no global send/receive switches and never sends the phone clipboard automatically.
- A phone's receive permission permits history retrieval into the app. It does not authorize a computer to disclose content or overwrite the phone clipboard. The computer must also allow sending to that phone under its own global and per-device settings.
- Every permitted computer may provide discovery information independently. Discovering another computer does not authorize access to it. Allowing one direction or one device grants no permission in the reverse direction or to a third device.
- Distinguish a network device, a compatible application, an authenticated identity and permission to obtain content. Show only states that are actually known; another device's permission may be unknown until it answers a request.
- Device names may be identical or change. Users need enough identity information to distinguish devices and perform the existing identity confirmation before allowing outgoing content. A changed device key is a new identity and does not inherit sending permission.
- Enabling a computer's sending permission allows the phone to request all currently retained local-origin history permitted by the computer's content/history settings. This includes copies made before permission was granted or while desktop automatic sending was off. This consequence must be understandable when access is granted.
- Revoking access prevents new authorized requests or transfers. It cannot recall content already obtained by another device. Existing fetched phone content may remain until local retention or clearing removes it.
- Support independent device send/receive and text/image restrictions. A setting must appear saved only after the actual operation succeeds.

## Explicit clipboard operations

Opening or resuming the app, discovering devices, retrieving history and viewing previews must not read or write the phone clipboard or send its current contents.

Importing the phone clipboard requires explicit user intent and creates a fixed snapshot. Sending that snapshot requires a separate explicit user action. The user must be able to understand its content and eligible destinations before committing to the transfer. A subsequent clipboard change must not silently replace the imported content. Current permissions are checked when sending.

Putting a remote history item on the phone clipboard also requires explicit user intent. Fetching or viewing it does not constitute a clipboard write. Remote history is never automatically forwarded. Resending retained phone-origin content uses the selected item and current permissions.

Backgrounding discards an unsent imported draft and stops active synchronization work. Reopening must not silently import, restore an unsent draft, or replay a send. Completed or interrupted operations must retain truthful results where available. System-controlled paste consent and platform failures must be respected.

## History and content understanding

Users need a coherent account of recent copies from multiple computers and their own phone-origin sends. Support locating records by source, direction and content type without prescribing a particular organization or filtering control.

For each record, the experience must communicate the relevant source, copy time, content type, preview/body availability and action readiness. Phone-origin sends must retain the outcomes for each destination. Source freshness and incomplete retrieval must remain understandable when results from different computers are combined.

Repeated retrieval of the same copy event must not create duplicates. Separate copy events with identical content remain separate. Different devices' clocks can disagree, so the app cannot guarantee exact chronology across computers.

Distinguish these meanings:

| Known fact | What it establishes |
| --- | --- |
| A history record was found | Its metadata is known; the full content may still be unavailable. |
| Content is available in the app | The retained body can be used for preview and an explicit local copy. |
| A phone clipboard copy was requested | The operation may succeed, fail or remain unverified according to platform evidence. |
| A destination confirmed a send | That destination reported its result; other destinations may have different results. |

One source failing must not hide useful results from others. Available cached content may remain usable while its source is offline. Missing, expired or evicted content cannot be previewed, copied or resent; never substitute another item or the current clipboard. Historical success does not guarantee current content availability or another app's ability to paste it.

## Retention and privacy

- Recent history is bounded and configurable, with a default of 20 events. Support history disabled, status only and retained content, including a zero history limit. Status-only records have no retained body for preview or reuse.
- Text and the history list live in process memory. Images may use encrypted temporary storage with bounded quotas and cleanup. Process restart clears session history/content; permitted remote history can be fetched again later.
- History disabled prevents phone history queries and caching. Retention limits, remote restarts, remote history settings and time spent offline can leave gaps. The product is not a permanent archive or guaranteed offline queue.
- Clearing local history removes app history, drafts and its history image cache. It does not clear the system clipboard, other applications' copies, other devices' history or completed transfers. Already exported clipboard content has a separate platform lifetime. Remote retained copies can reappear after a later permitted refresh.
- Transport is authenticated and encrypted between devices, with no central clipboard store. Avoid promises of absolute security or complete erasure everywhere.
- Content must not appear in system notification bodies or thumbnails. If notifications are offered, they are optional and denying them must not disable the main functions.
- Use fictional content, images, names and identities in design artifacts. Keep real clipboard data and device details out of shared files.

## States requiring design coverage

The design must let users understand each state below, what remains usable and any supported recovery. The designer chooses the placement, feedback mechanism and wording. Show progress only when actual progress information is available.

| Area | Required coverage |
| --- | --- |
| Prerequisites | Tailscale not installed/not connected, compatible computer app unavailable, unsupported application version. |
| Discovery | Waiting for discovery, no directly connected computer, partial discovery, reconnecting, a newly authenticated device. |
| Access | Local sending/receiving disabled, remote access denied or unknown, content type restricted, no eligible send destination. |
| History retrieval | Initial update, incremental update, last checked information by source, partial results, one source unavailable while others succeed. |
| Empty or absent history | No retained events, data not yet fetched, history disabled, status-only data, clearing/restart and retention gaps. |
| Clipboard import | Empty clipboard, paste permission refused, unsupported/corrupt/oversized content, image source unavailable, import in progress and a captured snapshot ready for use. |
| Transfers | In progress, confirmed success, failure, cancellation, unknown result and mixed results across destinations. |
| Phone clipboard copy | In progress, confirmed copy, failure or unverified copy; subsequent paste happens in another app. |
| Content availability | Body loading, content ready, missing/expired body and a source offline with or without usable cached content. |
| Lifecycle and settings | Background interruption, discarded draft, failed setting save, failed cleanup and recovery supported by the app. |

Avoid claims that all history has been recovered, every device is synchronized, an unknown transfer succeeded or fetched content is already on the phone clipboard.

## Platform and quality requirements

Design for iPhone and Android phones using their native accessibility, lifecycle and clipboard conventions. iOS system paste controls and privacy prompts constrain clipboard import. Copied images on both platforms must be usable by other apps through the platform's clipboard support. The design must allow for platform differences and expose failures honestly.

Support English, Dutch, German and French, plus following the system language. Deliver light and dark appearances, large text, VoiceOver/TalkBack semantics, suitable touch targets, long content/device names, localized text expansion and reduced motion. Status must remain understandable without color alone.

Users must be able to find the actual installed app version for support and compatibility checks. The design team chooses its placement.

The app should be lightweight, with bounded previews and history. Idle and background states must not imply continuous work or require sustained animation. Use the established Web brand and logo as context and prepare native mobile app icon assets. The design team selects the visual language and its adaptation to phone use.

## Design deliverables and review criteria

Provide editable design sources, the chosen information architecture and journeys, interactive prototypes for principal tasks, a reusable component/state library, final microcopy prepared for localization, accessibility notes and implementation behavior notes. Include native app icon exports and their editable sources using the existing brand assets. Record any behavior that requires a new product capability before it becomes an implementation requirement.

Review the proposal against these outcomes:

1. A user distinguishes receiving history into the app, copying to the phone clipboard and sending from the phone.
2. A user understands the captured content and effective destinations before authorizing a send.
3. Independent device permissions and access to already retained computer history are understandable.
4. Multiple sources, incomplete history and partial/unknown transfer results remain clear.
5. Reopening retrieves permitted retained history without silently accessing the clipboard or replaying sends.
6. Retention, clearing, restart and background limitations are accurately communicated.
7. Both platforms, supported languages, accessibility and privacy requirements are covered.

## Engineering ownership

Product behavior is tracked in [iOS #51](https://github.com/KatInBroek/shuttli/issues/51), [Android #52](https://github.com/KatInBroek/shuttli/issues/52) and [shared SDK #50](https://github.com/KatInBroek/shuttli/issues/50). Engineering uses the [mobile feature specification](../mobile/feature-spec.md) and platform acceptance specifications. This brief is the designer-facing requirement source; engineering screen/action examples do not constrain the design team's chosen presentation.
