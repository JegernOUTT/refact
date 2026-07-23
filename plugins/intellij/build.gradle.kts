import org.jetbrains.intellij.platform.gradle.IntelliJPlatformType
import org.jetbrains.intellij.platform.gradle.TestFrameworkType
import org.jetbrains.intellij.platform.gradle.tasks.VerifyPluginTask

plugins {
    id("java") // Java support
    alias(libs.plugins.kotlin) // Kotlin support
    alias(libs.plugins.intelliJPlatform) // IntelliJ Platform Gradle Plugin
    alias(libs.plugins.changelog) // Gradle Changelog Plugin
    alias(libs.plugins.qodana) // Gradle Qodana Plugin
    alias(libs.plugins.kover) // Gradle Kover Plugin
}

group = providers.gradleProperty("pluginGroup").get()
version = providers.environmentVariable("REFACT_BUILD_VERSION").orNull
    ?.takeIf { it.isNotBlank() }
    ?: getVersionString(providers.gradleProperty("pluginVersion").get())

val javaCompilerVersion = "17"
kotlin {
    jvmToolchain(javaCompilerVersion.toInt())
}

repositories {
    mavenCentral()

    intellijPlatform {
        defaultRepositories()
    }
}

dependencies {
    implementation("dev.gitlive:kotlin-diff-utils:5.0.7")
    implementation("org.apache.httpcomponents.client5:httpclient5:5.3.1") {
        exclude("org.slf4j")
    }
    implementation("org.jetbrains.kotlin:kotlin-reflect:1.8.10")
    implementation("com.vladsch.flexmark:flexmark-all:0.64.8")
    implementation("io.github.kezhenxu94:cache-lite:0.2.0")

    // test libraries
    testImplementation(kotlin("test"))
    testImplementation("com.google.code.gson:gson:2.10.1")
    testImplementation("com.squareup.okhttp3:mockwebserver3:5.0.0-alpha.14")
    testImplementation("org.bouncycastle:bcpkix-jdk15on:1.68")
    testImplementation("org.mockito:mockito-core:5.10.0")
    testImplementation("org.mockito.kotlin:mockito-kotlin:5.2.1")

    intellijPlatform {
        create(providers.gradleProperty("platformType"), providers.gradleProperty("platformVersion"))

        // Plugin Dependencies. Uses `platformBundledPlugins` property from the gradle.properties file for bundled IntelliJ Platform plugins.
        bundledPlugins(providers.gradleProperty("platformBundledPlugins").map { it.split(',') })

        // Plugin Dependencies. Uses `platformPlugins` property from the gradle.properties file for plugin from JetBrains Marketplace.
        plugins(providers.gradleProperty("platformPlugins").map { it.split(',') })

        pluginVerifier()
        zipSigner()
        testFramework(TestFrameworkType.Platform)
    }
}

intellijPlatform {
    pluginConfiguration {
        ideaVersion {
            sinceBuild = providers.gradleProperty("pluginSinceBuild")
            val pluginUntilBuild = providers.gradleProperty("pluginUntilBuild").orNull
            if (!pluginUntilBuild.isNullOrBlank()) {
                untilBuild = pluginUntilBuild
            }
        }
    }

    signing {
        certificateChain = providers.environmentVariable("CERTIFICATE_CHAIN")
        privateKey = providers.environmentVariable("PRIVATE_KEY")
        password = providers.environmentVariable("PRIVATE_KEY_PASSWORD")
    }

    publishing {
        token = providers.environmentVariable("PUBLISH_TOKEN")
        channels = providers.environmentVariable("PUBLISH_CHANNEL").map { listOf(it) }
    }

    pluginVerification {
        // INTERNAL_API_USAGES is reported as a warning (not a failure) so the plugin can
        // declare an unbounded until-build and stay available on future IDEs (262+). Real
        // breakage is still caught by COMPATIBILITY_PROBLEMS / INVALID_PLUGIN.
        failureLevel = listOf(
            VerifyPluginTask.FailureLevel.COMPATIBILITY_PROBLEMS,
            VerifyPluginTask.FailureLevel.INVALID_PLUGIN,
        )
        ides {
            recommended()
        }
    }
}

val runIdeWith2025 by intellijPlatformTesting.runIde.registering {
    type = IntelliJPlatformType.PyCharmCommunity // or IdeaUltimate if you use IU
    version = "2025.1"
    useInstaller = false
}

val runIdeWith2025JcefWorkaround by intellijPlatformTesting.runIde.registering {
    type = IntelliJPlatformType.PyCharmCommunity
    version = "2025.1"
    useInstaller = false
}

tasks.named("runIdeWith2025") {
    (this as JavaExec).jvmArgs("-Xmx4096m")
}

tasks.named("runIdeWith2025JcefWorkaround") {
    (this as JavaExec).jvmArgs(
        "-Xmx4096m",
        "-Dide.browser.jcef.out-of-process.enabled=false",
    )
}

// Configurable via: -PrunIdeType=IntellijIdeaUltimate -PrunIdeVersion=2024.3.5
// Available types: IntellijIdeaCommunity, IntellijIdeaUltimate, PyCharmCommunity,
//                  PyCharmProfessional, CLion, GoLand, WebStorm, PhpStorm, Rider, RustRover
val runIdeCustom by intellijPlatformTesting.runIde.registering {
    type = IntelliJPlatformType.valueOf(
        providers.gradleProperty("runIdeType").getOrElse("IntellijIdeaUltimate")
    )
    version = providers.gradleProperty("runIdeVersion").getOrElse("2025.1")
    useInstaller = false
}

tasks {
    // Set the JVM compatibility versions
    withType<JavaCompile> {
        sourceCompatibility = javaCompilerVersion
        targetCompatibility = javaCompilerVersion
    }
    withType<org.jetbrains.kotlin.gradle.tasks.KotlinCompile> {
        compilerOptions.jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.fromTarget(javaCompilerVersion))
    }

    // Isolate tests from the developer's real daemon home so they never register
    // /tmp/unitTest_* projects in ~/.cache/refact/daemon/projects.json.
    withType<Test> {
        val isolatedDaemonHome = layout.buildDirectory.dir("test-daemon-home").get().asFile
        systemProperty("refact.daemon.dir", isolatedDaemonHome.absolutePath)
        environment("REFACT_DAEMON_DIR", isolatedDaemonHome.absolutePath)
        doFirst { isolatedDaemonHome.mkdirs() }
    }

    // Bundled engine binaries must be loose files in the plugin directory (refact/bin/...),
    // not resources inside the jar, so RefactBinaryResolver can execute them directly.
    processResources {
        exclude("bin/**")
    }
    withType<org.jetbrains.intellij.platform.gradle.tasks.PrepareSandboxTask> {
        from(layout.projectDirectory.dir("src/main/resources/bin")) {
            into(pluginName.map { "$it/bin" })
        }
    }
}

fun runCommandOrNull(cmd: String): String? {
    return try {
        providers.exec {
            isIgnoreExitValue = true
            commandLine(cmd.split(" "))
        }.standardOutput.asText.get().trim().takeIf { it.isNotEmpty() }
    } catch (_: Exception) {
        null
    }
}

fun runCommand(cmd: String): String {
    return runCommandOrNull(cmd) ?: ""
}

fun getVersionString(baseVersion: String): String {
    val releaseTagVersion = runCommand("git tag -l --points-at HEAD")
        .lines()
        .mapNotNull { Regex("^release/v(\\d+\\.\\d+\\.\\d+)$").matchEntire(it)?.groupValues?.get(1) }
        .singleOrNull()

    if (System.getenv("PUBLISH_EAP") != "1" && releaseTagVersion == baseVersion) return baseVersion

    val branch = runCommand("git rev-parse --abbrev-ref HEAD")
        .ifEmpty { "unknown" }
        .replace("/", "-")
    val numberOfCommits = if (branch == "main") {
        val lastTag = runCommandOrNull("git describe --tags --abbrev=0 --match v* @^")
        if (lastTag != null) {
            runCommand("git rev-list ${lastTag}..HEAD --count")
        } else {
            runCommand("git rev-list --count HEAD")
        }
    } else {
        runCommandOrNull("git rev-list --count HEAD ^origin/main")
            ?: runCommand("git rev-list --count HEAD")
    }
    val commitId = runCommand("git rev-parse --short=8 HEAD")
    return if (System.getenv("PUBLISH_EAP") == "1") {
        "$baseVersion.$numberOfCommits-eap-$commitId"
    } else {
        "$baseVersion-$branch-$numberOfCommits-$commitId"
    }
}
