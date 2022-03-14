contract c1{
    // Send    coins
	// as foo
	function foo() {
		bar();
		a = t();
		b(c());
		bar();
	}

	// oh no!
	/* one line comment*/
	/* multi line
	   comment */
	function bar() {
		no(1, 2, 3);
	}
}