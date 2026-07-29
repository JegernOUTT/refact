package com.smallcloud.refactai.lsp

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.file.Files
import java.nio.file.Path
import java.util.Comparator

class RefactPathRegistrarTest {

    // ----------------------------------------------------------------------------------------
    // Profile selection
    // ----------------------------------------------------------------------------------------

    @Test
    fun selectsZprofileForMacZsh() {
        val home = Path.of("/home/user")
        val kind = RefactPathRegistrar.shellKind("/bin/zsh")
        val profile = RefactPathRegistrar.unixProfileFile(home, "Mac OS X", kind)
        assertEquals(home.resolve(".zprofile"), profile)
    }

    @Test
    fun selectsBashProfileForMacBash() {
        val home = Path.of("/home/user")
        val kind = RefactPathRegistrar.shellKind("/bin/bash")
        val profile = RefactPathRegistrar.unixProfileFile(home, "Mac OS X", kind)
        assertEquals(home.resolve(".bash_profile"), profile)
    }

    @Test
    fun selectsZshrcForLinuxZsh() {
        val home = Path.of("/home/user")
        val kind = RefactPathRegistrar.shellKind("/usr/bin/zsh")
        val profile = RefactPathRegistrar.unixProfileFile(home, "Linux", kind)
        assertEquals(home.resolve(".zshrc"), profile)
    }

    @Test
    fun selectsBashrcForLinuxBash() {
        val home = Path.of("/home/user")
        val kind = RefactPathRegistrar.shellKind("/bin/bash")
        val profile = RefactPathRegistrar.unixProfileFile(home, "Linux", kind)
        assertEquals(home.resolve(".bashrc"), profile)
    }

    @Test
    fun selectsFishConfig() {
        val home = Path.of("/home/user")
        val kind = RefactPathRegistrar.shellKind("/usr/bin/fish")
        val profile = RefactPathRegistrar.unixProfileFile(home, "Linux", kind)
        assertEquals(home.resolve(".config").resolve("fish").resolve("config.fish"), profile)
    }

    @Test
    fun selectsFishConfigFromXdgHome() {
        val home = Path.of("/home/user")
        val xdgHome = home.resolve("custom-config")
        val kind = RefactPathRegistrar.shellKind("/usr/bin/fish")
        val profile = RefactPathRegistrar.unixProfileFile(home, "Linux", kind, xdgHome)
        assertEquals(xdgHome.resolve("fish").resolve("config.fish"), profile)
    }

    @Test
    fun fallsBackToProfileForUnknownShell() {
        val home = Path.of("/home/user")
        val kind = RefactPathRegistrar.shellKind("/bin/tcsh")
        val profile = RefactPathRegistrar.unixProfileFile(home, "Linux", kind)
        assertEquals(home.resolve(".profile"), profile)
    }

    // ----------------------------------------------------------------------------------------
    // Add / update / idempotence
    // ----------------------------------------------------------------------------------------

    @Test
    fun addsBlockWhenMissing() {
        withTempHome { home ->
            val result = RefactPathRegistrar.register(home, "Linux", "/bin/bash")
            assertTrue(result is RegistrationResult.Changed)

            val content = Files.readString(home.resolve(".bashrc"))
            assertTrue(content.contains(REFACT_PATH_MARKER_BEGIN))
            assertTrue(content.contains(REFACT_PATH_MARKER_END))
            assertTrue(content.contains(".refact"))
        }
    }

    @Test
    fun secondRegistrationIsNoOp() {
        withTempHome { home ->
            val first = RefactPathRegistrar.register(home, "Linux", "/bin/bash")
            assertTrue(first is RegistrationResult.Changed)
            val afterFirst = Files.readString(home.resolve(".bashrc"))

            val second = RefactPathRegistrar.register(home, "Linux", "/bin/bash")
            assertTrue(second is RegistrationResult.Unchanged)
            assertEquals(afterFirst, Files.readString(home.resolve(".bashrc")))
        }
    }

    @Test
    fun replacesStaleOwnedContent() {
        withTempHome { home ->
            val profile = home.resolve(".bashrc")
            Files.createDirectories(profile.parent)
            Files.writeString(
                profile,
                "# user content\n$REFACT_PATH_MARKER_BEGIN\nexport PATH=\"/old/path:\$PATH\"\n$REFACT_PATH_MARKER_END\n# tail\n"
            )

            val result = RefactPathRegistrar.register(home, "Linux", "/bin/bash")
            assertTrue(result is RegistrationResult.Changed)

            val content = Files.readString(profile)
            assertTrue(content.contains("# user content"))
            assertTrue(content.contains("# tail"))
            assertFalse(content.contains("/old/path"))
            assertTrue(content.contains(".refact"))
            // exactly one owned block remains
            assertEquals(1, content.split(REFACT_PATH_MARKER_BEGIN).size - 1)
            assertEquals(1, content.split(REFACT_PATH_MARKER_END).size - 1)
        }
    }

    @Test
    fun preservesUserContentWhenAppending() {
        withTempHome { home ->
            val profile = home.resolve(".bashrc")
            Files.createDirectories(profile.parent)
            Files.writeString(profile, "alias foo=bar\n")

            RefactPathRegistrar.register(home, "Linux", "/bin/bash")

            val content = Files.readString(profile)
            assertTrue(content.contains("alias foo=bar"))
            assertTrue(content.contains(REFACT_PATH_MARKER_BEGIN))
        }
    }

    // ----------------------------------------------------------------------------------------
    // Malformed markers
    // ----------------------------------------------------------------------------------------

    @Test
    fun malformedMarkersReturnWarningAndDoNotTouchFile() {
        withTempHome { home ->
            val profile = home.resolve(".bashrc")
            Files.createDirectories(profile.parent)
            val original = "# user\n$REFACT_PATH_MARKER_BEGIN\nexport PATH=\"/x:\$PATH\"\n"
            Files.writeString(profile, original)

            val result = RefactPathRegistrar.register(home, "Linux", "/bin/bash")
            assertTrue(result is RegistrationResult.Warning)
            assertEquals(original, Files.readString(profile))
        }
    }

    @Test
    fun extractOwnedBlockDetectsBalancedBlock() {
        val content = "a\n$REFACT_PATH_MARKER_BEGIN\nline\n$REFACT_PATH_MARKER_END\nb"
        val block = RefactPathRegistrar.extractOwnedBlock(content)
        assertTrue(block != null)
        assertFalse(block!!.malformed)
    }

    @Test
    fun extractOwnedBlockReturnsNullWhenAbsent() {
        assertNull(RefactPathRegistrar.extractOwnedBlock("nothing here"))
    }

    @Test
    fun extractOwnedBlockFlagsUnbalancedMarkers() {
        val onlyBegin = "$REFACT_PATH_MARKER_BEGIN\nline"
        assertTrue(RefactPathRegistrar.extractOwnedBlock(onlyBegin)!!.malformed)

        val reversed = "$REFACT_PATH_MARKER_END\nline\n$REFACT_PATH_MARKER_BEGIN"
        assertTrue(RefactPathRegistrar.extractOwnedBlock(reversed)!!.malformed)
    }

    // ----------------------------------------------------------------------------------------
    // Windows
    // ----------------------------------------------------------------------------------------

    @Test
    fun windowsSuccessReturnsChangedWithoutTouchingRegistry() {
        val runner = RecordingWindowsRunner(WindowsPathCommandResult(exitCode = 0, changed = true))
        val result = RefactPathRegistrar.register(Path.of("C:\\Users\\me"), "Windows 11", null, runner)
        assertTrue(result is RegistrationResult.Changed)
        assertEquals(1, runner.calls.size)
        assertTrue(runner.calls.first().toString().endsWith("bin"))
    }

    @Test
    fun windowsAlreadyPresentReturnsUnchanged() {
        val runner = RecordingWindowsRunner(WindowsPathCommandResult(exitCode = 0, changed = false))
        val result = RefactPathRegistrar.register(Path.of("C:\\Users\\me"), "Windows 11", null, runner)
        assertTrue(result is RegistrationResult.Unchanged)
    }

    @Test
    fun windowsFailureReturnsWarning() {
        val runner = RecordingWindowsRunner(WindowsPathCommandResult(exitCode = 1))
        val result = RefactPathRegistrar.register(Path.of("C:\\Users\\me"), "Windows 11", null, runner)
        assertTrue(result is RegistrationResult.Warning)
    }

    @Test
    fun windowsRunnerExceptionIsCapturedAsWarning() {
        val runner = object : WindowsPathCommandRunner {
            override fun addToUserPath(binDir: Path): WindowsPathCommandResult = throw RuntimeException("boom")
        }
        val result = RefactPathRegistrar.register(Path.of("C:\\Users\\me"), "Windows 11", null, runner)
        assertTrue(result is RegistrationResult.Warning)
    }

    @Test
    fun powerShellScriptIsCaseInsensitiveAndIdempotent() {
        val script = PowerShellWindowsPathCommandRunner.buildPowerShellScript()
        assertTrue(script.contains("-ieq"))
        assertTrue(script.contains("if (-not \$exists)"))
        assertTrue(script.contains("SetEnvironmentVariable"))
        assertTrue(script.contains("[Console]::In.ReadToEnd()"))
        assertTrue(script.contains("FromBase64String"))
        assertFalse(script.contains("C:\\Users"))
    }

    // ----------------------------------------------------------------------------------------
    // Shared-path policy (mirrors LSPProcessHolder canonical check)
    // ----------------------------------------------------------------------------------------

    @Test
    fun sharedPathPolicyMatchesCanonicalOnUnix() {
        val home = Path.of("/home/user")
        val canonical = sharedRefactBinaryPath(home, "Linux").toString()
        assertTrue(pathMatchesShared(canonical, canonical, "Linux"))
        assertFalse(pathMatchesShared("/usr/local/bin/refact", canonical, "Linux"))
    }

    @Test
    fun sharedPathPolicyIsCaseInsensitiveOnWindows() {
        val home = Path.of("C:\\Users\\me")
        val canonical = sharedRefactBinaryPath(home, "Windows 11").toString()
        assertTrue(pathMatchesShared(canonical.uppercase(), canonical, "Windows 11"))
        assertFalse(pathMatchesShared("C:\\other\\refact.exe", canonical, "Windows 11"))
    }

    // ----------------------------------------------------------------------------------------
    // Helpers
    // ----------------------------------------------------------------------------------------

    private class RecordingWindowsRunner(private val result: WindowsPathCommandResult) : WindowsPathCommandRunner {
        val calls = mutableListOf<Path>()
        override fun addToUserPath(binDir: Path): WindowsPathCommandResult {
            calls.add(binDir)
            return result
        }
    }

    /** Reproduces the shared-path comparison used in LSPProcessHolder.binaryPathForDaemon. */
    private fun pathMatchesShared(resolved: String, canonical: String, osName: String): Boolean {
        val isWindows = osName.lowercase().contains("win")
        val resolvedNormalized = runCatching {
            Path.of(resolved).toAbsolutePath().normalize().toString()
        }.getOrDefault(resolved)
        val canonicalNormalized = runCatching {
            Path.of(canonical).toAbsolutePath().normalize().toString()
        }.getOrDefault(canonical)
        return if (isWindows) {
            resolvedNormalized.equals(canonicalNormalized, ignoreCase = true)
        } else {
            resolvedNormalized == canonicalNormalized
        }
    }

    private fun withTempHome(block: (Path) -> Unit) {
        val home = Files.createTempDirectory("refact-path-registrar")
        try {
            block(home)
        } finally {
            home.deleteRecursively()
        }
    }

    private fun Path.deleteRecursively() {
        if (!Files.exists(this)) return
        Files.walk(this).use { paths ->
            paths.sorted(Comparator.reverseOrder()).forEach { Files.deleteIfExists(it) }
        }
    }
}
