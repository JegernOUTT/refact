package com.smallcloud.refactai.lsp

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test
import java.nio.file.AtomicMoveNotSupportedException
import java.nio.file.Files
import java.nio.file.Path
import java.nio.file.StandardCopyOption
import java.util.Comparator

class RefactBinaryResolverTest {
    @Test
    fun binaryPromotionFallsBackWhenAtomicMoveIsUnsupported() {
        val root = Files.createTempDirectory("refact-binary-resolver-atomic-fallback")
        val source = root.resolve("source")
        val target = root.resolve("target")
        val attempts = mutableListOf<Boolean>()
        try {
            Files.writeString(source, "new-binary")
            Files.writeString(target, "old-binary")

            moveReplacingWithAtomicFallback(source, target) { from, to, atomic ->
                attempts.add(atomic)
                if (atomic) {
                    throw AtomicMoveNotSupportedException(from.toString(), to.toString(), "unsupported")
                }
                Files.move(from, to, StandardCopyOption.REPLACE_EXISTING)
            }

            assertEquals(listOf(true, false), attempts)
            assertEquals("new-binary", Files.readString(target))
            assertFalse(Files.exists(source))
        } finally {
            root.deleteRecursively()
        }
    }
}

private fun Path.deleteRecursively() {
    if (!Files.exists(this)) return
    Files.walk(this).use { paths ->
        paths.sorted(Comparator.reverseOrder()).forEach { Files.deleteIfExists(it) }
    }
}
