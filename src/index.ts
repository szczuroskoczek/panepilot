import { registerHotkey, Modifiers, openWebview } from "$native";

registerHotkey(Modifiers.Alt, 0x41, () => {
	const wv = openWebview('test');

	setTimeout(() => {
		wv.exit();
	}, 2500);
});
