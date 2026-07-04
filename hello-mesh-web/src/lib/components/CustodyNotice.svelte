<!--
	Persistent, always-visible honesty label (bead c3f, hard requirement, not
	optional copy): a browser-generated identity is SOFT custody — extractable
	by the node serving this page — never equivalent to app/hardware (hard)
	custody. See docs/network-coordination/user-identity.md §3, §4.2.
-->
<p
	class="flex items-start gap-2 rounded-md border border-amber-800/40 bg-amber-950/30 px-3 py-2 text-sm text-amber-200"
>
	<span aria-hidden="true">⚠</span>
	<span>
		<strong class="font-semibold">Soft custody.</strong> A browser-created identity's key lives in
		this browser and can be read by whichever node is currently serving this page. It is
		<em>not</em> equivalent to an app or hardware key you alone hold. Fine for a disposable, low-value
		identity — never for anything you'd regret someone else touching.
	</span>
</p>
