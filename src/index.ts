import {
	registerHotkey,
	Modifiers,
	openWebview,
	registerAltRelease,
} from "$native";

const wv = openWebview("test", 400, 400);

let visible = false;
registerHotkey(Modifiers.Alt, 0x41, () => {
	wv.setHtml(`
		<html>
		<head>
		</head>
		<body>
		<div>Hello World ${Math.random()}</div>
		</body>
		</html>
		`);
	if (visible) return;
	wv.setVisible(true);
	visible = true;
	registerAltRelease(() => {
		wv.setVisible(false);
		visible = false;
	});
});
