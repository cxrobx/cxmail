# CXMail iOS

CXMail's existing blue accent (#2f6fde); system neutral grays, semantic grouped canvas and label colors.
System typography and native controls. Radius scale: Capsule, 8 (chips), 12
(message surfaces), 16 (chat surfaces); continuous corners only. No custom shadows.
Native Mail is the navigation reference: mailbox -> message push, a modal composer,
native swipe/context actions, large collapsing titles, bottom reader actions.
Honor all safe-area edges: the Duo's 84-point status column is not usable space.
Liquid Glass is explicitly requested: use native glass on compact navigation,
account/filter chrome and floating action capsules, with neutral depth behind it.
Mail rows stay dense and unboxed; email documents keep their own formatting.
Open wide and flat: narrower inbox, wider reader. Partially folded: two panes,
the active 40-point division kept free. Open tall: full-width reader with an
inbox drawer over it, initially closed. Closed: mailbox-to-message navigation.
Selection, draft and chat state live above adaptive branches.
The split reader has no redundant navigation heading: the subject is its title.
The iOS icon uses the actual desktop CXMail artwork, not a generic envelope.
Mail chat and compose actions are separate 48pt glass icon buttons. Chat follows
T3's composer/transcript structure: multiline text above an internal send/stop
control, a centered reading column, neutral user bubbles and unboxed Markdown
assistant responses. Keep tool activity collapsed and permission controls explicit.
Docked chat uses native columns with a draggable divider, at least 300pt per pane.
Preserve the active division; narrow windows return to floating chat. Conversation,
unsent input and split preference survive docking and popping out.
