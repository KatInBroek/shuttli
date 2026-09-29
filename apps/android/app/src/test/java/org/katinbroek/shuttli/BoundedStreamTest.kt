package org.katinbroek.shuttli

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertNull
import org.junit.Test
import java.io.ByteArrayInputStream

class BoundedStreamTest {
    @Test fun acceptsExactLimitAndRejectsOneByteMore() {
        val exact = ByteArray(131072) { (it % 251).toByte() }
        assertArrayEquals(exact, ByteArrayInputStream(exact).readBytesBounded(exact.size))
        assertNull(ByteArrayInputStream(exact + 1.toByte()).readBytesBounded(exact.size))
    }
}
