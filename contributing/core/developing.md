# Developing

- The development branch is `canary`.
- All pull requests should be opened against `canary`.
- The changes on the `canary` branch are published to the `@canary` tag on npm regularly.

To develop locally:

1. Install Rust and Cargo via [rustup](https://rustup.rs).
1. Install the [GitHub CLI](https://github.com/cli/cli#installation).
1. Clone the Next.js repository (download only recent commits for faster clone):
   ```
   gh repo clone vercel/next.js -- --filter=blob:none --branch canary --single-branch
   ```
1. Create a new branch:
   ```
   git checkout -b MY_BRANCH_NAME origin/canary
   ```
1. Enable pnpm:
   ```
   corepack enable pnpm
   ```
1. Install the dependencies with:
   ```
   pnpm install
   ```
1. Start developing and watch for code changes:
   ```
   pnpm dev
   ```
1. In a new terminal, run `pnpm types` to compile declaration files from
   TypeScript.
   _Note: You may need to repeat this step if your types get outdated._
1. When your changes are finished, commit them to the branch:
   ```
   git add .
   git commit -m "DESCRIBE_YOUR_CHANGES_HERE"
   ```
1. To open a pull request you can use the GitHub CLI which automatically forks and sets up a remote branch. Follow the prompts when running:
   ```
   gh pr create
   ```

For instructions on how to build a project with your local version of the CLI,
see **[Developing Using Your Local Version of Next.js](./developing-using-local-app.md)** as linking the package is not sufficient to develop locally.

## Testing a local Next.js version on an application

Since Turbopack doesn't support symlinks when pointing outside of the workspace directory, it can be difficult to develop against a local Next.js version. Neither `pnpm link` nor `file:` imports quite cut it. An alternative is to pack the Next.js version you want to test into a tarball and add it to the pnpm overrides of your test application. The following script will do it for you:

```bash
pnpm pack-next --release && pnpm unpack-next path/to/project
```

Or without running the build:

```bash
pnpm pack-next --no-build --release && pnpm unpack-next path/to/project
```

### Explanation of the scripts

```bash
# Generate a tarball of the Next.js version you want to test
$ pnpm pack-next

# If you need to build in release mode:
$ pnpm pack-next --release
# You can also pass any cargo argument to the script

# To skip the `pnpm i` and `pnpm build` steps in next.js (e. g. if you are running `pnpm dev`)
$ pnpm pack-next --no-build
```

Afterwards, you'll need to unpack the tarball into your test project. You can either manually edit the `package.json` to point to the new tarballs (see the stdout from `pack-next` script), or you can automatically unpack it with:

```bash
# Unpack the tarballs generated with pack-next into project's node_modules
$ pnpm unpack-next path/to/project
```
