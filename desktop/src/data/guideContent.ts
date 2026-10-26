export type GuideLevel = 'basic' | 'advanced';

export interface GuideCallout {
  tone: 'tip' | 'note' | 'warning';
  title: string;
  text: string;
}

export interface GuideSection {
  title: string;
  body: string;
  steps?: string[];
  bullets?: string[];
  callout?: GuideCallout;
}

export interface GuidePage {
  summary: string;
  outcomes: string[];
  sections: GuideSection[];
}

export interface GuideTopic {
  id: string;
  title: string;
  shortTitle: string;
  category: 'Start' | 'Play' | 'Manage' | 'Recover' | 'Customize' | 'Connect';
  description: string;
  keywords: string[];
  basic: GuidePage;
  advanced: GuidePage;
}

export interface GuideOverview {
  title: string;
  body: string;
  terms: { term: string; definition: string }[];
  callout: GuideCallout;
}

/** Orientation shown at the top of the app's Home page and the website's docs home. */
export const GUIDE_OVERVIEW: GuideOverview = {
  title: 'What Agora manages',
  body: 'Agora looks after the files and settings around modded Minecraft: instances, mods, packs, Java runtimes, snapshots, compatibility checks, and recovery. By default, the official Minecraft launcher still runs the game and handles your Microsoft account.',
  terms: [
    {
      term: 'Instance',
      definition: 'An isolated Minecraft setup with its own version, loader, mods, and settings.',
    },
    {
      term: 'Mod loader',
      definition: 'Software such as Fabric or Quilt that lets Minecraft load mods. Each instance has its own.',
    },
    {
      term: 'Catalog',
      definition: 'Agora\'s community-curated collection of mods, packs, and more. Downloading it turns on discovery and governance features.',
    },
  ],
  callout: {
    tone: 'note',
    title: 'Your existing game is separate',
    text: 'Creating an Agora instance does not replace your normal Minecraft installation. Changes stay inside the selected instance.',
  },
};

/**
 * Written for players, not developers, on both the basic and advanced pages. Describe what the
 * player sees and can do, using the labels on screen; leave out how Agora works underneath unless
 * it changes what they should do. Suggest kindly ("we recommend", "it is worth") rather than
 * instruct, and prefer plain words over terms like artifact, endpoint, or transaction.
 */
export const GUIDE_TOPICS: GuideTopic[] = [
  {
    id: 'getting-started',
    title: 'Getting started with Agora',
    shortTitle: 'Getting started',
    category: 'Start',
    description: 'Set up Agora, understand what it does, and reach your first playable instance.',
    keywords: ['onboarding', 'setup', 'registry', 'java', 'first launch'],
    basic: {
      summary: 'Agora helps you discover, organize, and recover modded Minecraft installations. It calls each separate installation an instance and keeps risky changes contained to that instance.',
      outcomes: [
        'Complete the first-run setup without needing prior modding knowledge.',
        'Understand the difference between Agora, Minecraft, and the official launcher.',
        'Create a safe first instance and know where to go next.',
      ],
      sections: [
        {
          title: 'Onboarding',
          body: 'The onboarding wizard checks the optional services and local tools Agora can use. You can skip optional connections and change them later in Settings.',
          steps: [
            'Review the service switches. Leave Modrinth on for a larger live catalog, or off to use only Agora-curated data.',
            'Choose how Minecraft launches: delegated to the official Mojang launcher (the default, no sign-in required) or direct launch inside Agora.',
            'If you chose direct launch, let Agora prepare Java. Delegated launch skips this step because the official launcher brings its own runtime.',
            'Connect GitHub if you want to vote or participate in community governance. GitHub sign in also avoids rate limits, which can happen if you do a lot of searches and downloads.',
            'Download the signed catalog when prompted.',
            'Finish onboarding and open Browse to choose a pack, or My Instances to build an empty instance.',
          ],
        },
        {
          title: 'A safe first session',
          body: 'For the easiest start, choose a curated pack that matches a current Minecraft version. Packs select a tested group of mods for you and reduce the number of compatibility decisions you need to make.',
          steps: [
            'Open Browse and filter the content type to Pack.',
            'Open a pack, read its description and its Versions tab, then choose Create Instance from Pack.',
            'Keep Pack release selected: it installs the exact mod builds the curator tested. Flexible lets you pick another Minecraft version, but mods without a build for it are left out, and the install stops if a required one is missing.',
            'Set the instance name, then choose Check pack. It lists the Minecraft version and loader, how many mods will be installed, anything left out or missing, and each mod with its version. It does not show memory or download size; adjust memory afterwards in the instance editor.',
            'Launch the new instance. Resolve any health warnings before selecting Launch Anyway.',
            'Play for at least 60 seconds so Agora can save the last setup that launched fine (Last Known Good) as a recovery point.',
          ],
        },
      ],
    },
    advanced: {
      summary: 'Every choice you made during setup can be changed later. This page explains what each one does, so you can turn on only what you want.',
      outcomes: [
        'Know what each optional service is for.',
        'Know where to change your setup choices later.',
        'Check that everything is ready before building a big instance.',
      ],
      sections: [
        {
          title: 'What each connection is for',
          body: 'Agora works on its own with just its catalog. Everything else is optional, and you can turn each one on or off separately.',
          bullets: [
            'Modrinth adds many more mods to Browse. If it is on but nothing shows up, check that Modrinth is also allowed under Privacy.',
            'GitHub sign-in is only for voting and community curation. You never need it to download or play.',
            'Microsoft sign-in is only needed if you launch Minecraft from inside Agora. If the official launcher starts the game (the default), sign in there instead.',
          ],
        },
        {
          title: 'Before building a big instance',
          body: 'A quick check now can save you from a failed download halfway through a large pack.',
          steps: [
            'Open Settings. Turn on Advanced mode if you want to see Privacy and the more detailed Java options.',
            'Make sure the catalog says it is ready.',
            'Choose how Agora handles Java: Automatic is the easiest.',
            'If you have turned anything off under Privacy, make sure the things you need are still allowed.',
            'Try a small instance first and launch it once before importing a large pack.',
          ],
        },
        {
          title: 'Changing your mind later',
          body: 'Nothing you choose during setup is permanent. Settings has a section for each one: Services, Accounts, Launching, Java, Updates, and Privacy.',
          callout: {
            tone: 'tip',
            title: 'Playing without internet?',
            text: 'Download the catalog, your mods, and Java while you are still online, then launch once to make sure everything is there. The Privacy & offline guide walks through it.',
          },
        },
      ],
    },
  },
  {
    id: 'modding-foundations',
    title: 'Modding foundations',
    shortTitle: 'Modding basics',
    category: 'Start',
    description: 'Learn versions, loaders, dependencies, and the different kinds of Minecraft add-ons.',
    keywords: ['minecraft version', 'loader', 'fabric', 'quilt', 'dependencies', 'mods'],
    basic: {
      summary: 'Most modding problems come from mixing the wrong Minecraft version, loader, or dependency. Agora exposes these details so you can make compatible choices before files are changed.',
      outcomes: [
        'Read a compatibility label and version list confidently.',
        'Know what each content type changes.',
        'Recognize dependencies and conflicts before installing.',
      ],
      sections: [
        {
          title: 'The compatibility triangle',
          body: 'A mod normally has to match three things: the Minecraft version, the mod loader, and the versions of other mods it depends on. A Fabric mod for Minecraft 1.21 should not be assumed to work on Quilt, NeoForge, or Minecraft 1.20.1 unless its version page explicitly says so.',
          bullets: [
            'Minecraft version: the game release the file was built for.',
            'Loader: the platform that discovers and starts mods.',
            'Dependencies: other mods or libraries required by the selected file.',
          ],
        },
        {
          title: 'Know the content types',
          body: 'Agora can discover more than code mods. Select the content type that matches the result you want.',
          bullets: [
            'Mods change game code or behavior and usually require a loader.',
            'Packs create a complete instance from a coordinated set of content.',
            'Resource packs change textures, sounds, fonts, or models.',
            'Shaders change rendering and usually require a compatible shader-support mod.',
            'Data packs change recipes, loot, world generation, or game rules for worlds.',
            'Worlds provide playable world data; servers provide multiplayer destinations or community information.',
          ],
        },
        {
          title: 'Read compatibility before popularity',
          body: 'A popular project can still be wrong for your instance. When searching in the browse tab, select your instance in the top dropdown so Agora can filter the results to meet your instance.',
          callout: {
            tone: 'warning',
            title: 'Back up your instance',
            text: 'We always recommend making a backup of your instance before making significant changes.',
          },
        },
      ],
    },
    advanced: {
      summary: 'Once you have more than a handful of mods, a few habits make problems much easier to avoid and to track down.',
      outcomes: [
        'Know why one mod can bring several others with it.',
        'Understand why a mod can look compatible and still cause trouble.',
        'Add mods to a big instance without losing track of what changed.',
      ],
      sections: [
        {
          title: 'The exact version matters',
          body: 'Two people can both have "Sodium" installed and still have different files. When asking for help, mention the exact version you have, not just the mod\'s name.',
          bullets: [
            'Some mods need other mods to work. Agora adds those for you when you install.',
            'Optional extras add support for other mods. They are handy, but each one is another thing that could clash.',
            'Agora warns you about mods known to clash, whether the mod\'s author, a curator, or Agora\'s own checks spotted it.',
            'A file you add yourself may not carry as much information, so Agora can tell you less about it.',
          ],
        },
        {
          title: 'Versions',
          body: 'Agora works out which versions suit your instance. We recommend going with its suggestion unless you have a reason not to. When a mod has several versions, its Versions tab has the release notes and which Minecraft versions each one supports.',
        },
        {
          title: 'Change one thing at a time',
          body: 'On a big instance, adding lots of mods at once makes it hard to tell which one caused a problem. Adding a few at a time, and launching in between, keeps it easy. Loadout profiles can also switch groups of mods on and off without removing them.',
          steps: [
            'Start from a snapshot of a setup that launched fine.',
            'Install a few mods.',
            'Launch and try out the parts of the game they change.',
            'If stable, name a new snapshot or let a successful session mark the last setup that launched fine (Last Known Good).',
            'If something breaks, restore the snapshot and try a different version or fewer mods.',
          ],
        },
      ],
    },
  },
  {
    id: 'home-navigation',
    title: 'Home and navigation',
    shortTitle: 'Home & navigation',
    category: 'Start',
    description: 'Use the dashboard, sidebar, alerts, recommendations, and keyboard navigation.',
    keywords: ['home', 'sidebar', 'command palette', 'quick actions', 'recommendations'],
    basic: {
      summary: 'The Home page prioritizes what needs attention, what you played last, recovery options, and compatible discoveries. The sidebar keeps every major area one click away.',
      outcomes: [
        'Recognize each Home dashboard section.',
        'Move quickly between the catalog, instances, community, guide, and settings.',
        'Use the command palette without reaching for the mouse.',
      ],
      sections: [
        {
          title: 'Read Home from top to bottom',
          body: 'Alerts appear first because they may need action. Continue Playing launches the most recent instance. The last setup that launched fine (Last Known Good) offers recovery when one exists. Compatible curated matches use your recent instance version and loader.',
          bullets: [
            'A catalog alert means discovery data is missing or unavailable.',
            'A crash alert means the latest instance did not exit cleanly.',
            'Curated mod matches on the home page are filtered for your active instance when possible; they are not paid placements.',
          ],
        },
        {
          title: 'Use the sidebar',
          body: 'Home is the dashboard, Browse is the catalog, My Instances is your library, Community Governance shows curation activity, Help & Guide is this learning center, and Settings controls Agora itself.',
          steps: [
            'Select an item to move to that area.',
            'Use the arrow button at the sidebar edge to collapse or expand it.',
            'Hover over a collapsed icon to see its name.',
          ],
        },
        {
          title: 'Open Quick actions',
          body: 'Press Ctrl+K on Windows and Linux, or Cmd+K on macOS. Type an instance or destination name, use the arrow keys to select it, and press Enter. Escape closes the palette.',
          // callout: {
          //   tone: 'tip',
          //   title: 'Text fields keep their shortcuts',
          //   text: 'The palette shortcut is ignored while you are typing in an input, text area, or editable field.',
          // },
        },
      ],
    },
    advanced: {
      summary: 'A few extra ways to arrange Agora the way you like, and what the recovery card on Home actually restores.',
      outcomes: [
        'Resize and reset the sidebar.',
        'Know what restoring from Home does.',
      ],
      sections: [
        {
          title: 'Resize the sidebar',
          body: 'Drag the edge of the sidebar to make it wider or narrower. With the keyboard, focus the edge and press Left or Right (hold Shift to move further). Double-click the edge to put it back to its normal size. Agora remembers your choice.',
        },
        {
          title: 'The recovery card on Home',
          body: 'The card for the last setup that launched fine (Last Known Good) takes you back to how the instance was the last time it launched and ran properly, which is not always your newest snapshot. Restoring replaces the instance\'s current mods and settings, but Agora saves a snapshot of them first, so you can change your mind.',
          // callout: {
          //   tone: 'note',
          //   title: 'Recovery is deliberate',
          //   text: 'Restoring the last setup that launched fine (Last Known Good) creates an undo snapshot first, so the current state is not silently discarded.',
          // },
        },
        // {
        //   title: 'Preserved work context',
        //   body: 'Opening an item from Browse or the Instance Editor preserves the source page and scroll position. Use Back to return to the same working context. The browser-style back action is part of Agora\'s navigation history even though the app has no traditional web URLs.',
        // },
      ],
    },
  },
  {
    id: 'instances',
    title: 'Creating and managing instances',
    shortTitle: 'Instances',
    category: 'Play',
    description: 'Create isolated instances, edit their content, lock the stable ones, and manage their lifecycle.',
    keywords: ['instance', 'create', 'import', 'prism', 'curseforge', 'modrinth', 'edit', 'delete', 'lock', 'memory'],
    basic: {
      summary: 'An instance is a self-contained Minecraft setup. Separate instances let you keep different game versions, mod loaders, packs, and play styles without mixing their files.',
      outcomes: [
        'Create an instance with compatible core settings.',
        'Open, launch, rename, lock, and delete an instance safely.',
        'Know when to use separate instances instead of reconfiguring one.',
      ],
      sections: [
        {
          title: 'Create an instance',
          body: 'Open My Instances and select Create Instance. Give it a recognizable name, then choose a Minecraft version, loader, loader version, and memory allocation.',
          steps: [
            'Choose the Minecraft version required by the mods or pack you intend to use.',
            'Choose one loader. Do not mix loader-specific mod files.',
            'Keep the suggested loader version unless a project requires another.',
            'Start with a moderate memory allocation and adjust only when needed.',
            'Create the instance, then use Edit to add content.',
          ],
        },
        {
          title: 'Import from another launcher',
          body: 'Select Import Instances on My Instances to detect Prism Launcher, CurseForge, and Modrinth App profiles. Nothing is selected automatically: choose the instances you want, review disk use, loader support, and launch settings, then approve the batch once.',
          steps: [
            'Close Minecraft in the source launcher before copying a world.',
            'Review any unsupported loader or custom-component warning; Agora never substitutes a nearby version.',
            'Agora copies game data and leaves the source instance untouched. Accounts, credentials, and launch hooks are never imported.',
            'Successful instances remain imported even if another selected instance is skipped or fails.',
          ],
        },
        {
          title: 'Use instance cards',
          body: 'Each card shows the instance name, loader, Minecraft version, last launch, and lock state. Launch starts the game, Edit opens the full editor, Troubleshoot opens diagnostic tools, and Delete removes the Agora copy of the instance after confirmation. Anything you imported from another launcher is left alone, and the deleted instance waits in Recently deleted (below the cards) for 30 days, where you can Restore it or Delete it now.',
          callout: {
            tone: 'warning',
            title: 'Export before deleting',
            text: 'Snapshots are local recovery points, not an external backup. Recently deleted keeps an instance for 30 days, then moves it to your Recycle Bin; export anything you need to keep for good.',
          },
        },
        {
          title: 'Lock a stable instance',
          body: 'Locking prevents content changes while still allowing launch. Use it after a modpack or a personal instance is working well. Unlock only when you intend to install, remove, enable, disable, or update content.',
        },
        {
          title: 'Reuse instance settings with templates',
          body: 'Once an instance is set up the way you like, save it as a template: the memory and Java arguments, plus whichever configuration files you choose to include. Applying it to a new instance saves redoing the same work, and named Java profiles let you keep several memory setups without editing arguments each time.',
          callout: {
            tone: 'note',
            title: 'Templates carry settings, not mods',
            text: 'A template captures how an instance is configured, not what is installed in it. Use a pack export when you want to reproduce the content as well.',
          },
        },
      ],
    },
    advanced: {
      summary: 'Tips for keeping several instances organized, so a working setup stays working while you experiment elsewhere.',
      outcomes: [
        'Decide when to make a new instance instead of changing an old one.',
        'Use locks and snapshots together.',
        'Know what the instance page tells you.',
      ],
      sections: [
        {
          title: 'When to make a new instance',
          body: 'We recommend making a new instance when you change Minecraft version, switch loaders, try out a pack you are unsure about, or play on a server with its own mod list. Keeping these apart makes it much easier to undo a change or work out what broke.',
          bullets: [
            'Keep one locked instance you know works, just for playing.',
            'Try big updates and files you downloaded yourself in a spare test instance first.',
            'Clone from a lockfile when you need an exact copy.',
          ],
        },
        {
          title: 'Locking versus snapshots',
          body: 'A lock stops anyone, including you, from changing an instance\'s mods by accident. A snapshot saves a copy you can go back to. For an instance you care about, we suggest both: take a snapshot, then lock it. Unlocking later does not remove the snapshot.',
        },
        {
          title: 'What the instance page shows',
          body: 'The instance page shows whether the game is running, the game\'s log output, whether the instance is locked, and which Java it uses.',
          callout: {
            tone: 'warning',
            title: 'Kill is a last resort',
            text: 'Terminating a running game can lose unsaved game data. Exit Minecraft normally whenever possible.',
          },
        },
        {
          title: 'Update a prior import safely',
          body: 'Importing the same instance again updates the Agora copy, as long as you have not changed it since. If you have changed its mods, configs, saves, screenshots, or settings in Agora, the importer offers to make a second copy instead, so your changes are never overwritten.',
        },
      ],
    },
  },
  {
    id: 'browse-registry',
    title: 'Browse, search, and the catalog',
    shortTitle: 'Browse & catalog',
    category: 'Play',
    description: 'Find curated and live content, filter for compatibility, and understand trust labels.',
    keywords: ['browse', 'search', 'catalog', 'registry', 'curated', 'modrinth', 'sort'],
    basic: {
      summary: 'Browse combines Agora\'s curated catalog with optional live Modrinth results. Instance-aware discovery is the easiest way to avoid incompatible downloads.',
      outcomes: [
        'Search and filter the catalog effectively.',
        'Find content compatible with a specific instance.',
        'Understand Curated, Compatible, Major Match, and Installed labels.',
      ],
      sections: [
        {
          title: 'Start with the right scope',
          body: 'Use the content type filter for mods, packs, shaders, resource packs, servers, data packs, or worlds. Add a search term and category only when needed; overly narrow filters can hide valid results.',
          steps: [
            'Open Browse.',
            'Choose Discover for an instance and select the instance you plan to change.',
            'Choose a content type.',
            'Search by project name, feature, or category.',
            'Open a result to inspect its About, Gallery, Versions, and Agora information.',
          ],
        },
        {
          title: 'Read result badges',
          body: 'Compatible means Agora found an appropriate version for the selected Minecraft version and loader. Major Match is less exact and deserves testing. Installed means the selected instance already contains the project. Curated identifies a community-reviewed Agora catalog entry.',
        },
        {
          title: 'Choose a sort',
          body: 'For You uses overlap with your active instance. Net Score reflects community balance, Trending reflects recent voting velocity, Newest emphasizes recently added entries, and vote sorts expose the strongest positive or negative community signals.',
          callout: {
            tone: 'note',
            title: 'Curated and live results are different',
            text: 'A result without the Curated badge may still be legitimate, but it comes from the enabled live source rather than Agora\'s reviewed catalog.',
          },
        },
      ],
    },
    advanced: {
      summary: 'Where Browse results come from, and what to do when the catalog is missing or you are offline. Search order is a starting point, not a recommendation.',
      outcomes: [
        'Know the difference between curated and Modrinth results.',
        'Know what to do when Browse looks empty.',
        'Share a result so someone else gets the same thing.',
      ],
      sections: [
        {
          title: 'Where results come from',
          body: 'Curated results come from Agora\'s own catalog, which is downloaded to your computer. If you have turned Modrinth on, Browse also asks Modrinth while you search. A detail page can show both: Agora\'s notes alongside the latest description, pictures, and versions from Modrinth.',
          bullets: [
            'The downloaded catalog still works when you are offline.',
            'Modrinth may have newer versions than the catalog, but they have not been reviewed by Agora\'s community.',
            'Where a mod comes from tells you more than how its card looks.',
          ],
        },
        // {
        //   title: 'Audit before install',
        //   body: 'Check the license, source link, update date, supported loaders, exact game versions, dependency list, curator notes, reviews, and version changelog. Immunity indicates community protection from ordinary removal pressure; it is not a technical compatibility certification.',
        // },
        {
          title: 'When Browse looks empty',
          body: 'If the catalog is still loading, missing, or you are offline, Browse shows a message saying so. Read it before assuming nothing exists, and download or refresh the catalog if it offers. If you have turned things off under Privacy, you will only see what is already on your computer.',
          callout: {
            tone: 'tip',
            title: 'Sharing a find with a friend',
            text: 'Search results can be in a different order for someone else. Send them the exact version, or export a pack, so they get the same thing you did.',
          },
        },
      ],
    },
  },
  {
    id: 'install-update',
    title: 'Installing, removing, and updating mods',
    shortTitle: 'Install & update',
    category: 'Play',
    description: 'Install, update, and remove mods, with a review before anything changes and a way to undo it.',
    keywords: ['install flow', 'dependencies', 'remove', 'updates', 'rollback', 'conflicts'],
    basic: {
      summary: 'Before Agora changes anything, it shows you what an install will add and asks you to confirm. If something goes wrong, you can roll back to how the instance was before.',
      outcomes: [
        'Install a mod into the right instance.',
        'Know what to look at on the review screen.',
        'Update or remove mods and still be able to undo it.',
      ],
      sections: [
        {
          title: 'Install from a detail page',
          body: 'Select Install to Instance, pick the instance, and choose a version. Agora then shows a review screen before anything changes. Packs work a little differently: they use Create Instance from Pack, which makes a new instance for them, and Check pack shows you the mods and versions first.',
          steps: [
            'Make sure the instance you picked is the one you meant.',
            'Pick a version marked as compatible when there is one.',
            'Look at what comes along with it. Some mods need others to work, and these are added for you. Optional extras are off unless you tick them.',
            'If the review says there is an overlap to sort out, choose what to do about it. Usually that means keeping what you have or replacing it.',
            'Select Install.',
          ],
          callout: {
            tone: 'tip',
            title: 'You can always go back',
            text: 'Agora saves a restore point right before it installs. If you do not like the result, select Roll Back on the result screen, or restore it later from the instance\'s Snapshots tab.',
          },
        },
        {
          title: 'Remove safely',
          body: 'We recommend removing mods with Remove on the Mods tab rather than deleting the file yourself. That way Agora can tell you if another mod needs it, and you can undo the removal later.',
          callout: {
            tone: 'warning',
            title: 'Dependents may stop loading',
            text: 'If another mod required the one you removed, Agora will ask you to remove it too. If you choose to keep the mod requiring the one you removed, that mod may fail to load or crash the game. Check the health report after removing mods.',
          },
        },
        {
          title: 'Check for updates',
          body: 'On My Instances, select Edit on the instance, open its Mods tab, and select Check for updates. Resource Packs, Shaders and Data Packs have the same button. Update one item from its row, or use Update All to review every update in that tab at once.',
          bullets: [
            'A locked instance cannot be updated until you unlock it.',
            'A newer version of one mod can still clash with your others, so it is worth launching once after updating.',
          ],
        },
        {
          title: 'Agora keeps an eye out for updates',
          body: 'After you check once, Agora remembers what it found, even after a restart. Instances with updates waiting show a badge, and Agora checks again on its own from time to time. You can change how often in Settings.',
          callout: {
            tone: 'note',
            title: 'Only offered when a build actually fits',
            text: 'Agora prefers to show you only mod versions that are exactly compatible with your instance. If a mod is for a different minor Minecraft version, it may be offered, but it is not guaranteed to work. Mods for significantly different loaders or major Minecraft versions are unlikely to work, and therefore are not offered.',
          },
        },
        {
          title: 'See what changed, and pin what should not',
          body: 'When the author has written release notes, Agora shows them before you update, so you can see what changed. If you want to stay on the version you have, open the three-dot menu at the end of the mod\'s row and choose Pin updates. Update All will leave it alone, but you can still update it yourself whenever you like. The same menu item unpins it.',
        },
      ],
    },
    advanced: {
      summary: 'A closer look at the review screen: what each part means, what happens when something goes wrong, and how to make big changes without getting lost.',
      outcomes: [
        'Understand each part of the review screen.',
        'Know what to do if an install leaves problems behind.',
        'Update a large instance in manageable steps.',
      ],
      sections: [
        {
          title: 'Reading the review screen',
          body: 'The review screen is laid out with the things you need to decide at the top.',
          bullets: [
            'Comes along with it: mods the one you chose needs in order to work. These are added for you.',
            'Optional extras: mods that add something extra, such as support for another mod. Tick any you want.',
            'Overlaps to sort out: something you already have clashes with what you are installing. Pick an option for each one before you can continue.',
            'Curator warnings and Worth knowing: things the catalog\'s curators or Agora think you should read first. They do not stop the install.',
            'Technical details: the full list of files that will be added, removed, or turned off, if you are curious.',
          ],
        },
        {
          title: 'If something goes wrong',
          body: 'After installing, Agora checks the instance for problems such as a missing required mod. If it finds one, the result screen lists what is wrong and keeps the new files in place so you can see it for yourself. From there you can fix it, or select Roll Back to put the instance back to how it was. If the install fails partway through, Agora undoes it for you.',
          callout: {
            tone: 'note',
            title: 'A clean check does not mean it is tested',
            text: 'Agora can spot missing mods and known clashes, but it cannot tell whether everything plays well together. We recommend launching and trying out the new mod before you rely on the instance.',
          },
        },
        {
          title: 'Update in smaller groups',
          body: 'On a big instance, updating everything at once makes it hard to tell which update caused a new problem. We suggest updating a few related mods at a time, such as a library and the mods that use it.',
          steps: [
            'Take a snapshot of the instance while it works.',
            'Update a handful of related mods.',
            'Launch and play for a bit.',
            'If all is well, move on to the next group. If not, roll back and try fewer at once.',
          ],
        },
      ],
    },
  },
  {
    id: 'content-management',
    title: 'Managing mods and game content',
    shortTitle: 'Manage content',
    category: 'Manage',
    description: 'Organize mods, resource packs, shaders, and data packs inside an instance.',
    keywords: ['mods tab', 'resource packs', 'shaders', 'data packs', 'enable', 'disable', 'manual jar'],
    basic: {
      summary: 'The Instance Editor shows everything installed in an instance, sorted by type. Making changes here, rather than in the folder, means Agora can warn you about problems and help you undo them.',
      outcomes: [
        'Find and manage each kind of installed content.',
        'Temporarily disable a mod without deleting it.',
        'Add local files with appropriate caution.',
      ],
      sections: [
        {
          title: 'Use the content tabs',
          body: 'Open My Instances, select Edit, then choose Mods, Resource Packs, Shaders, or Data Packs. Each list shows the installed file and available actions for that content type.',
          bullets: [
            'View Details returns to the project page when Agora can resolve the source.',
            'Disable keeps a mod file available but prevents it from loading.',
            'Remove starts a reviewed removal plan.',
            'Add opens Browse with the current instance and content type already selected.',
          ],
        },
        {
          title: 'Enable and disable for testing',
          body: 'Disabling is useful when diagnosing a conflict or keeping optional features available for later. Relaunch after changing enabled state; Minecraft cannot unload most mods from a running game.',
        },
        {
          title: 'How data packs reach your worlds',
          body: 'Minecraft only loads data packs from inside each world, not from the instance. Agora keeps the instance\'s data pack list and adds/removes them from your worlds when you launch or change that list.',
          bullets: [
            'By default a data pack goes to all worlds. Select "Worlds…" on its row to pick specific worlds instead.',
            'The row shows where it is, for example All worlds (3) or 2 of 3 worlds.',
            'A world created while you play gets your data packs at the next sync, so a brand-new world has them from its second session. Select "Sync now" on the Data Packs tab to add them sooner.',
            'Agora only ever removes or replaces data pack files it placed itself. If a world already has a different file with the same name, Agora leaves it alone and says so.',
          ],
        },
        {
          title: 'Import a local file',
          body: 'Use Import Mod or the drop zone for a local mod file. Use the corresponding content picker for supported resource packs, shaders, and data packs. Confirm the file came from a trusted source and matches the instance.',
          callout: {
            tone: 'warning',
            title: 'Manual files have fewer guarantees',
            text: 'Agora knows less about a file you add yourself, so it cannot warn you as much. We suggest taking a snapshot first, and only adding files from sources you trust.',
          },
        },
        {
          title: 'Group a long list',
          body: 'A large instance is easier to read the list of mods grouped, rather than only alphabetically. In the content toolbar, next to the search and filter controls, the Group dropdown re-sorts the list into sections. Each group header has its own enable and disable button, so you can switch a whole set at once.',
          steps: [
            'Open My Instances, select Edit, then a content tab such as Mods.',
            'Use the Group dropdown in the toolbar above the list.',
            'Choose "Modpack vs Added by You", "Category", or "Source".',
            'Collapse the sections you are not working on.',
          ],
          callout: {
            tone: 'tip',
            title: 'Pack content is marked',
            text: 'In an instance created from a modpack, Group: Modpack vs Added by You separates what the pack brought from what you added. That same distinction is what lets a pack update leave your additions alone.',
          },
        },
        {
          title: 'Make your own groups',
          body: 'The built-in groupings are derived from what Agora already knows. To sort mods your own way, open the three-dot menu at the end of a row and choose Set group. Name a new group or pick an existing one. Once at least one group exists, Group: My groups appears in the dropdown alongside the built-in options.',
          callout: {
            tone: 'note',
            title: 'One mod at a time',
            text: 'Group assignment is per row from its own menu. The bulk selection bar covers enabling, disabling, and removing, but not grouping.',
          },
        },
      ],
    },
    advanced: {
      summary: 'Find out why a mod is installed, tidy up leftovers, and avoid surprises in your worlds when removing content.',
      outcomes: [
        'Spot mods Agora cannot update for you.',
        'Find out why an unfamiliar mod is installed.',
        'Remove content without damaging your worlds.',
      ],
      sections: [
        {
          title: 'Read the Mods list',
          body: 'The Mods list shows each mod\'s name, its file name, and where and when it was installed, when Agora knows. If Agora cannot tell where a mod came from, such as a file you added yourself, it may not be able to update it for you, so you may need to update it by hand.',
        },
        {
          title: 'Ask why a mod is here',
          body: 'Open the three-dot menu at the end of a row and choose Why is this here. Agora traces that file back and answers with one of four things: you installed it directly, it came from the instance\'s modpack, another mod needs it, or it was a dependency and nothing needs it any more. It then lists the installed mods that currently require it. Most launchers show what a mod depends on; this is the opposite direction, which is the one you are looking in when you find a JAR you do not recognise.',
          callout: {
            tone: 'tip',
            title: 'Ask before you remove',
            text: 'An unfamiliar library is usually something another mod asked for. Checking takes one click and is more reliable than judging by filename.',
          },
        },
        {
          title: 'Clean up orphaned dependencies',
          body: 'Sometimes a mod was only installed because another mod needed it. When you remove that other mod, Agora asks whether you want to remove the leftovers too. There is no separate cleanup screen to remember.',
          callout: {
            tone: 'note',
            title: 'Agora plays it safe',
            text: 'If any other mod might still use a leftover, Agora will not offer to remove it. Keeping an unused library does no harm. You can still check one yourself with Why is this here.',
          },
        },
        {
          title: 'Switch groups of mods on and off',
          body: 'Loadout profiles and the on/off switches let you keep, say, performance mods, visual mods, and big content mods as separate groups you can turn on together. Keep the libraries a mod needs switched on alongside it. We suggest taking a named snapshot before changing a core library or a mod that changes world generation.',
        },
        {
          title: 'Protect persistent game data',
          body: 'Removing a mod that adds blocks, items, storage, or world generation can leave holes in a world, even if the game still starts. We recommend backing up worlds you care about and trying the removal on a copy first.',
          callout: {
            tone: 'warning',
            title: 'Agora cannot check your worlds',
            text: 'Agora can tell whether your mods fit together, but not whether a world will be fine without a mod it used to have.',
          },
        },
      ],
    },
  },
  {
    id: 'launching',
    title: 'Launching Minecraft and health checks',
    shortTitle: 'Launch & health',
    category: 'Play',
    description: 'Choose how Minecraft starts, fix problems found before launch, and see what is happening while you play.',
    keywords: ['launch', 'delegated', 'direct', 'health check', 'console', 'running'],
    basic: {
      summary: 'Agora checks an instance before launch, then either hands it to the official launcher or starts Minecraft directly, depending on your Settings choice.',
      outcomes: [
        'Choose the appropriate launch mode.',
        'Respond safely to health warnings and blockers.',
        'Know where to find progress and console information.',
      ],
      sections: [
        {
          title: 'Choose delegated or direct launch',
          body: 'Delegated launch is the default: Agora gets everything ready, then the official Minecraft launcher starts the game. Direct launch starts the game from Agora itself, and needs you to sign in with Microsoft to play online.',
          bullets: [
            'Delegated launch is the simplest, since your account stays in the official launcher.',
            'Direct launch lets you watch the game\'s status and log output inside Agora.',
            'You can switch in Settings under Launching.',
          ],
        },
        {
          title: 'Handle the health screen',
          body: 'Before launching, Agora checks for problems. A blocker is something that will stop the game from working, so it is best to fix it first. A warning is a potential issue or concern that you should be aware of or fix. Agora may offer to turn off a problem mod for you right from the screen.',
          steps: [
            'Read what the problem is and which mod it is about.',
            'Fix missing mods or the wrong Java version first. These are the most common causes.',
            'Before turning off a mod, check whether other mods need it.',
            '"Launch Anyway" is offered if you want to try launching regardless. We suggest taking a snapshot first.',
          ],
        },
        {
          title: 'Known bad combinations',
          body: 'Some mods work fine on their own but break when used together. Agora\'s community keeps a list of these pairs, and Agora warns you before launch, or when you install something new, if your instance has one.',
          callout: {
            tone: 'note',
            title: 'Only the versions that clash are flagged',
            text: 'If two mods only clashed in certain versions, Agora only warns you about those versions.',
          },
        },
        {
          title: 'Fix a loader version mismatch',
          body: 'Sometimes a mod needs a newer or older version of your mod loader (such as Fabric or NeoForge) than the instance has. When that happens, the health screen tells you which mods are affected and suggests a loader version that suits them all.',
          steps: [
            'Read which mods need a different loader version.',
            'Select Switch and launch to change to the suggested version and start the game. If you are only reviewing, the same button is called Switch version.',
            'If you would rather pick a version yourself, use Choose compatible version.',
            'If no single loaderversion suits every mod installed, Agora cannot switch loaders for you. You may need to remove or update one of the mods instead.',
            'Some versions ask you to confirm before switching, because Agora could not fully check them.',
            'After switching, Agora checks the instance again.',
          ],
          callout: {
            tone: 'tip',
            title: 'Changing the loader version yourself',
            text: 'You can change the loader version any time with the Change loader version button next to the loader name in the instance editor. When an install needs a different loader version, the review screen offers the same choice, or lets you skip the mods that do not fit.',
          },
        },
        {
          title: 'During and after launch',
          body: 'The first launch of an instance may take a little while as Agora downloads Java and loader files, and you will see its progress. With direct launch, Agora shows that the game is running and its log output. When you are done, we recommend quitting from inside Minecraft rather than closing the window, so your worlds save properly.',
        },
      ],
    },
    advanced: {
      summary: 'This section covers what the pre-launch check can and cannot catch, how to quiet warnings you have already looked at, and how to grab useful logs when something goes wrong.',
      outcomes: [
        'Know what the health check covers.',
        'Mute warnings without hiding new problems.',
        'Find the useful part of the game\'s log.',
      ],
      sections: [
        {
          title: 'What the health check can catch',
          body: 'The check before launch can spot mods known to clash, missing mods, and the wrong Java version. It cannot tell you how the game will behave once it is running, such as graphics driver problems, damaged worlds, or bugs that only show up while you play.',
        },
        {
          title: 'Muting warnings',
          body: 'If you have read a warning and are happy to ignore it, you can mute it so it stops appearing for that mod. If every warning is muted, Agora launches straight away. After a big update, it is worth looking at your muted warnings again, since they may matter now.',
          callout: {
            tone: 'warning',
            title: 'Muting is not fixing',
            text: 'Muting only hides the message. We suggest muting only warnings you understand, and keeping a snapshot in case you were wrong.',
          },
        },
        {
          title: 'Getting useful logs',
          body: 'With direct launch, the Console tab shows the game\'s log. Filter it to All, Warnings+, or Errors, or switch to Raw to see every line as written, then copy what you need. The first error is usually the most useful. The last few lines are often just the game shutting down because of it. If there is a crash report too, include it.',
        },
      ],
    },
  },
  {
    id: 'crash-recovery',
    title: 'Crash diagnosis and recovery',
    shortTitle: 'Crashes & recovery',
    category: 'Recover',
    description: 'Use Crash Doctor, evidence-based testing, and recovery to the last setup that launched fine (Last Known Good).',
    keywords: ['crash doctor', 'crash report', 'suspect', 'disable relaunch', 'restore'],
    basic: {
      summary: 'Crashes are usually solved fastest by preserving the current state, reading the first useful evidence, and testing one likely cause at a time. Crash Doctor guides that process.',
      outcomes: [
        'Open an automatic or pasted-log investigation.',
        'Test a ranked suspect without losing the starting state.',
        'Restore a known-good instance when investigation is not the priority.',
      ],
      sections: [
        {
          title: 'Start an investigation',
          body: 'A recent crash can appear as a Home alert or on an instance card. Open Troubleshoot or paste a log when automatic detection cannot find the report. Crash Doctor ranks likely suspects from available signals.',
        },
        {
          title: 'Test the top suspect',
          body: 'Select Disable & Relaunch. Agora creates a recovery snapshot before changing the enabled state. After testing, choose Yes, fixed or Still crashing so the investigation can record the result and continue.',
          steps: [
            'Read the suspect name and evidence summary.',
            'Review any dependent mods that would also be affected.',
            'Disable the smallest safe set and relaunch.',
            'Reproduce the action that caused the crash.',
            'Report the result accurately rather than changing several other things first.',
          ],
        },
        {
          title: 'When no single mod looks guilty',
          body: 'If the evidence does not point anywhere, use guided bisect. Agora disables half your mods, you launch and report whether it still crashes, and it halves the remaining candidates each round until one is left. A handful of launches will find a culprit that no crash log named.',
          callout: {
            tone: 'note',
            title: 'Each half is kept loadable',
            text: 'Bisect never splits a mod away from something it depends on. A trial that crashed on a missing dependency would tell you about the split rather than the bug, so the halves are chosen to stay launchable. If you answer a round wrongly, you can step back and take the other half.',
          },
        },
        {
          title: 'Recover instead of investigate',
          body: 'If you need to play immediately, restore the last setup that launched fine (Last Known Good) from Home or the Snapshots tab. Use Restore All & Close in Crash Doctor to undo its test changes.',
          callout: {
            tone: 'tip',
            title: 'Keep the crash report',
            text: 'Restoring files may remove the immediate problem, but the original report is still valuable when reporting the incompatibility upstream.',
          },
        },
      ],
    },
    advanced: {
      summary: 'Why Crash Doctor suspects a mod, how to test it without muddying the results, and what to include when asking someone else for help.',
      outcomes: [
        'Understand why Crash Doctor picked a suspect.',
        'Test one thing at a time so the answer is clear.',
        'Put together a helpful report when you cannot solve it alone.',
      ],
      sections: [
        {
          title: 'Why a mod is a suspect',
          body: 'Crash Doctor looks at several clues: whether the mod appears in the crash log, whether it is known to clash with something you have, what other mods depend on it, and whether it has caused crashes before. A high score means "worth testing first", not "definitely guilty". Mods you have already ruled out stay on the list so you can see what you have tried.',
        },
        {
          title: 'Test one thing at a time',
          body: 'When Crash Doctor turns off a mod, it may also need to turn off the mods that depend on it. Try not to change anything else at the same time, such as Java, the loader, or other mods, or you will not know which change fixed it. Once the crash stops, make sure it really is fixed before turning other things back on.',
        },
        {
          title: 'Asking for help',
          body: 'People can help much faster when you include: your Minecraft and loader versions, the crash report, what you were doing when it crashed, whether it also crashes in a new world, and what you have already tried. A lockfile lets them see your exact mod list.',
          callout: {
            tone: 'note',
            title: 'Sharing a report sends it to someone else',
            text: 'Copy crash report in Crash Doctor gives you a report with your file paths and username removed, ready to paste into Discord or an AI assistant. Whatever you paste goes to that service, so give it a quick read first for anything private, such as server addresses.',
          },
        },
      ],
    },
  },
  {
    id: 'snapshots-loadouts',
    title: 'Snapshots and loadout profiles',
    shortTitle: 'Snapshots & loadouts',
    category: 'Recover',
    description: 'Create restore points and switch between groups of enabled mods.',
    keywords: ['snapshot', 'last known good', 'last setup that launched fine', 'loadout', 'profile', 'diff', 'restore'],
    basic: {
      summary: 'Snapshots restore files to an earlier state. Loadout profiles remember which mods, resource packs, shader packs, data packs and worlds are switched on. They solve different problems and work well together.',
      outcomes: [
        'Create, compare, restore, and delete snapshots.',
        'Understand how the last setup that launched fine (Last Known Good) is saved automatically.',
        'Save and apply a loadout profile, and know what it covers.',
      ],
      sections: [
        {
          title: 'Create a snapshot',
          body: 'Open an instance, select Snapshots, enter a useful label, and create the snapshot before a risky change. Include the reason in the label, such as Before 1.21.2 update or Stable server instance.',
        },
        {
          title: 'Compare and restore',
          body: 'Show diff lists which files were added, removed, or changed since the snapshot. Restore puts the instance back to how it was, and saves a snapshot of the current state first in case you change your mind. Delete removes just that one snapshot. Snapshots you make yourself can include your worlds. The automatic ones Agora takes before each launch leave worlds out so launching stays quick.',
          callout: {
            tone: 'note',
            title: 'The last setup that launched fine is saved automatically',
            text: 'After you play for at least 60 seconds without a crash, Agora remembers that setup as the last setup that launched fine (Last Known Good). It covers your mods and settings, not your worlds.',
          },
        },
        {
          title: 'Create a loadout profile',
          body: 'Open Loadout Profiles, give the profile a name, and save it. It remembers which mods, resource packs, shader packs, data packs and worlds are switched on or off right now. Applying it later switches everything back to match.',
          bullets: [
            'If you have added a mod since saving the profile, Agora asks whether to keep it on or switch it off when you apply.',
            'After adding or removing mods, use Update from setup on the profile to save the instance as it is now. Agora shows you what will change first.',
            'Applying a profile saves an undo point under Snapshots called before-loadout, so you can go back.',
            'A profile does not change configs, mod versions, or the loader.',
            'Relaunch Minecraft after applying a profile.',
          ],
        },
        {
          title: 'Export and import a backup',
          body: 'You can export a snapshot to a single file to keep somewhere safe or move to another computer. Importing one checks the file first, then adds it to that instance\'s list of snapshots.',
          callout: {
            tone: 'note',
            title: 'Importing never overwrites what you have',
            text: 'Importing only adds the snapshot to the list. Agora then offers to restore it, which you can do once the game is closed. As with any restore, Agora saves a snapshot of the current state first.',
          },
        },
      ],
    },
    advanced: {
      summary: 'Getting more out of snapshots and profiles together, and why you still want a backup of your worlds.',
      outcomes: [
        'Make sense of a snapshot diff.',
        'Use snapshots and profiles together.',
        'Know what snapshots do not protect.',
      ],
      sections: [
        {
          title: 'Reading a diff',
          body: 'A diff lists every file that was added, removed, or changed. Turning a mod off shows up as its file being removed and a .disabled copy being added, which is normal. Changed config files can matter even when the mod list looks the same.',
        },
        {
          title: 'Using snapshots and profiles together',
          body: 'Take a snapshot of a setup you know works, then make profiles for different ways to play, such as one with extra visuals and one that suits a multiplayer server. Profiles only switch things on and off. They do not change mod versions or settings, so restore the snapshot when you want those back as well.',
        },
        {
          title: 'Keep your own backups too',
          body: 'Snapshots and profiles are stored on this computer, alongside the instance. They are not a backup somewhere else and do not sync anywhere. To be able to rebuild an instance elsewhere, export a pack or lockfile, and back up worlds you cannot replace separately.',
          callout: {
            tone: 'warning',
            title: 'A lockfile is not a world backup',
            text: 'A lockfile lists your mods and settings, but does not contain your worlds or config files.',
          },
        },
      ],
    },
  },
  {
    id: 'packs-sharing',
    title: 'Packs, import, export, and reproduction',
    shortTitle: 'Packs & sharing',
    category: 'Manage',
    description: 'Move complete instances between machines and verify that another instance matches.',
    keywords: ['pack', 'mrpack', 'import', 'export', 'lockfile', 'clone', 'repair', 'friend', 'another computer', 'transfer', 'share', 'send'],
    basic: {
      summary: 'A pack is the easiest way to share a whole setup with someone else. Agora can open Modrinth packs (.mrpack) and its own pack format.',
      outcomes: [
        'Import a supported pack into a new instance.',
        'Choose the right export format.',
        'Share an instance without assuming snapshots travel with it.',
      ],
      sections: [
        {
          title: 'Import a pack',
          body: 'On My Instances, click Import pack file at the top and pick a .mrpack or an Agora pack file. (In an instance\'s editor, the Import pack as new instance button at the top does the same, and the Other launchers & files tab has Select File & Import for a .mrpack or Prism .zip.) Either kind of pack opens a review before anything is created: the new instance\'s name, every file with where it comes from (a Modrinth or other download, or a copy inside the pack), what is left out, and anything that would stop the import.',
          steps: [
            'Choose a pack file from someone you trust.',
            'In the review, check the instance name. If one with that name already exists, Agora suggests a numbered copy such as "Name (2)"; edit it if you like. The existing instance is never overwritten.',
            'Read the list of files and what is left out, then choose Import. Cancel leaves nothing behind.',
            'Wait while Agora downloads and sets everything up.',
            'Launch once to check the pack works before adding more mods.',
          ],
        },
        {
          title: 'Update a pack without losing your changes',
          body: 'When the pack author publishes a new version, Agora compares three things: what the old pack shipped, what you have now, and what the new pack ships. Files the pack changed and you did not are simply updated. Mods you added yourself are kept. Where the pack changed something you also edited, Agora stops and asks rather than choosing for you.',
          steps: [
            'Point Agora at the new pack file to see a preview before anything changes.',
            'Review the files it will add, update, and remove.',
            'Answer every conflict — there is no default for a file you both edited.',
            'Apply. A snapshot is taken first and the whole update rolls back if it fails.',
          ],
          callout: {
            tone: 'note',
            title: 'Your edits are the ones that need a decision',
            text: 'A conflict means the author changed a file you had also changed. Keeping yours means missing their fix; taking theirs means losing your edit. Only you know which matters, so Agora refuses to guess.',
          },
        },
        {
          title: 'Send a setup to a friend or another computer',
          body: 'Open the instance with Edit, go to the Export tab, and choose Export as Modrinth Pack (.mrpack). It works for any setup: mods Agora can find on Modrinth are listed with download links, and every other mod is included as a file inside the pack, so nothing is missing on the other end. Your resource packs, shader packs and datapacks go along too (linked when Modrinth hosts that exact file, otherwise included as files), because you chose them. Disabled mods are included and arrive disabled. Send the saved file however you like. Configs, keybinds, and settings are not included, and neither are saves or snapshots.',
          steps: [
            'On the Export tab, pick Export as Modrinth Pack (.mrpack) and choose where to save it.',
            'Send the file to your friend, or copy it to the other computer.',
            'In Agora there, go to My Instances, click Import pack file, and pick the file. Prism, Modrinth App and other launchers can open a .mrpack too.',
            'Review the name and the file list, choose Import, let Agora finish preparing, then launch once to check it.',
          ],
          callout: {
            tone: 'note',
            title: 'The Agora pack (.json) is lighter but needs sources',
            text: 'The Agora pack lists mods without including them, so each mod needs a catalog or Modrinth source to be installed on the other side. If some do not have one, Agora names them before saving. Choose .mrpack to send those files along; the Agora pack never carries resource packs, shader packs or datapacks. Disabled mods stay disabled on the other side, and the import review marks them. The pack carries the instance name you see in Agora.',
          },
        },
        {
          title: 'Choose an export',
          body: 'The Export tab offers Export as Modrinth Pack (.mrpack) and Export as Agora Pack (.json). You choose where to save the file, and afterwards Show in folder takes you straight to it.',
        },
        {
          title: 'Sharing with others',
          body: 'Let people know which Minecraft and loader versions your pack is for. Worlds, server lists, and most settings do not come along. If you plan to share the pack publicly, check that each mod\'s license allows it to be shared.',
          callout: {
            tone: 'tip',
            title: 'Try it yourself first',
            text: 'Before sending a pack around, import it into a fresh instance to check it works.',
          },
        },
      ],
    },
    advanced: {
      summary: 'A lockfile is an exact list of an instance\'s files. It is handy for checking that two instances match, fixing one that has drifted, or showing someone exactly what you have when asking for help.',
      outcomes: [
        'Know when to use a lockfile instead of a pack.',
        'Check an instance against a lockfile and fix differences.',
        'Share an instance that shows a problem so others can see it too.',
      ],
      sections: [
        {
          title: 'What a lockfile is',
          body: 'A lockfile lists every mod file in an instance, where it came from, and a few key settings, so another instance can be checked against it. It does not include your config files. You can export, copy, or paste one from the Export tab.',
        },
        {
          title: 'Verify, repair, or clone',
          body: 'Verify shows how your instance differs from the lockfile. Repair makes your instance match it, downloading what is missing and removing extras. It takes a snapshot first and undoes itself if the result has problems. Clone makes a brand-new instance that matches the lockfile and leaves yours alone.',
          steps: [
            'Only use a lockfile from someone you trust.',
            'Run Verify and look through what is missing, extra, or different.',
            'If you have changes you want to keep, take a snapshot before Repair.',
            'Choose Clone if you would rather not touch your existing instance.',
          ],
        },
        {
          title: 'Sharing a problem with someone',
          body: 'If you want help with a bug, it helps to cut the instance down to as few mods as still show the problem, export a lockfile, and write down the steps that cause it. If a world or log is needed, check it for anything private before sharing.',
          callout: {
            tone: 'tip',
            title: 'Matching files are not proof of safety',
            text: 'Verify can tell you two files are identical, but not whether the original was safe to use.',
          },
        },
      ],
    },
  },
  {
    id: 'java-performance',
    title: 'Java, memory, and performance',
    shortTitle: 'Java & performance',
    category: 'Customize',
    description: 'Select a compatible runtime and tune memory and garbage collection without guesswork.',
    keywords: ['java', 'jvm', 'memory', 'ram', 'garbage collection', 'g1gc', 'zgc'],
    basic: {
      summary: 'Each Minecraft version needs a particular version of Java. Agora can find or download the right one for you and pick sensible performance settings on its own.',
      outcomes: [
        'Use the right Java for each instance.',
        'Give Minecraft enough memory without slowing down your computer.',
        'Know why the automatic settings are a good default.',
      ],
      sections: [
        {
          title: 'Let Agora manage Java',
          body: 'With Java set to Automatic, Agora uses a suitable Java already on your computer or downloads one. Java it downloads is kept just for Agora and does not affect other programs. Choose Prompt if you want Agora to ask before downloading, or Manual if you prefer to install Java yourself.',
        },
        {
          title: 'Set memory sensibly',
          body: 'More memory is not always faster. We suggest giving Minecraft enough for the pack while leaving a few gigabytes for everything else on your computer. If the game feels slow, more memory is not always the fix. It helps most when the game runs out of memory or the log says it is short.',
          callout: {
            tone: 'tip',
            title: 'Start with the pack recommendation',
            text: 'Small instances often need much less memory than large modpacks. Change one step at a time and observe actual behavior.',
          },
        },
        {
          title: 'Leave performance tuning on Auto',
          body: 'Java has settings that affect how smoothly the game runs, and Auto picks good ones for your Java version. We recommend leaving it on Auto unless you have a specific reason to change it.',
        },
      ],
    },
    advanced: {
      summary: 'For when you want to choose Java or its settings yourself for one instance. The automatic choices suit most people, so it is worth having a reason before changing them.',
      outcomes: [
        'Use a different Java for one instance.',
        'Compare the performance options fairly.',
        'Avoid old Java settings that cause trouble.',
      ],
      sections: [
        {
          title: 'Use a different Java for one instance',
          body: 'Settings decides which Java Agora uses everywhere. To use a different one for a single instance, pick it on that instance\'s Java & Args tab. When you are done testing, clear it again so Agora goes back to choosing a suitable Java for you.',
        },
        {
          title: 'Trying the performance options',
          body: 'Auto is the default. ZGC low latency can reduce sudden pauses on newer Java versions. G1GC high efficiency is another option you can pick yourself. Pre-touch allocated memory sets aside all the game\'s memory at startup, which can reduce stutter later but makes the game slower to start.',
          bullets: [
            'To compare fairly, test in the same world doing the same things.',
            'Watch for stutter, how long the game takes to start, how much memory it uses, and whether it crashes.',
          ],
        },
        {
          title: 'Use your own Java arguments sparingly',
          body: 'Arguments you add yourself can clash with the ones Agora sets, or be out of date. The launch preview shows what will actually be used. Allowing an incompatible Java version is best kept for a quick test, with a snapshot to go back to.',
          callout: {
            tone: 'warning',
            title: 'Java settings from guides online age quickly',
            text: 'A list of Java arguments written for an older Java version can cause problems on Java 17, 21, or 25. We recommend sticking with Auto.',
          },
        },
      ],
    },
  },
  {
    id: 'settings-appearance',
    title: 'Settings, appearance, and accessibility',
    shortTitle: 'Settings & appearance',
    category: 'Customize',
    description: 'Adjust Agora to your display, input, accessibility, and update preferences.',
    keywords: ['settings', 'appearance', 'theme', 'density', 'motion', 'contrast', 'updates'],
    basic: {
      summary: 'Settings changes how Agora looks and behaves. Appearance controls are independent from Minecraft graphics settings and can be reset without changing instances.',
      outcomes: [
        'Apply a coherent appearance preset.',
        'Adjust text, colors, motion, spacing, and sidebar layout.',
        'Find service, account, launch, Java, launcher, and update controls.',
      ],
      sections: [
        {
          title: 'Start with a preset',
          body: 'Choose an appearance preset closest to your needs, then adjust color mode, accent, font, density, corners, and text scale. Presets include default, night, compact, and high-readability options.',
        },
        {
          title: 'Improve comfort and access',
          body: 'Use the custom border color for stronger boundaries, Reduce motion to minimize animation, Text scale for typography, and Density for control spacing. These settings are separate so larger text does not have to make every panel excessively spacious.',
          callout: {
            tone: 'tip',
            title: 'System settings are respected',
            text: 'System color and motion modes can follow operating-system preferences. Full motion deliberately overrides reduced-motion behavior.',
          },
        },
        {
          title: 'Navigate the Settings page',
          body: 'Use the sticky section links to jump to Appearance, General, Services, Accounts, Launching, Java, Launcher, Updates, and Privacy when Advanced mode is enabled. Reset appearance or layout if experiments become difficult to read or navigate.',
        },
      ],
    },
    advanced: {
      summary: 'Make your own color scheme, and find the less common settings for the official launcher and Agora updates.',
      outcomes: [
        'Make custom colors that are still easy to read.',
        'Know what each appearance setting changes.',
        'Point Agora at the official launcher and keep Agora up to date.',
      ],
      sections: [
        {
          title: 'Make your own colors',
          body: 'Open Custom colors to change the block, navigation, background, and text colors, and how see-through panels are. Before keeping a scheme, look around a few pages to check that everything is still easy to read.',
          callout: {
            tone: 'tip',
            title: 'Check more than the main text',
            text: 'Faded text, links, borders, warning buttons, and the highlight around the selected item all need to stand out against your new colors too.',
          },
        },
        {
          title: 'What each appearance setting changes',
          body: 'Density changes the spacing around buttons and panels. Text scale changes the size of text. Corner style changes how rounded corners are, though round buttons stay round. Motion controls animations and smooth scrolling.',
        },
        {
          title: 'Launcher and update settings',
          body: 'If Agora cannot find the official Minecraft launcher on its own, set Launcher Path to where it is installed. Software Updates checks for a new version of Agora and can download it and restart for you. Reset Layout puts the sidebar back to its normal size without touching your instances.',
        },
      ],
    },
  },
  {
    id: 'accounts-services',
    title: 'Accounts and optional services',
    shortTitle: 'Accounts & services',
    category: 'Connect',
    description: 'Understand why Agora offers GitHub, Microsoft, and Modrinth connections.',
    keywords: ['account', 'github', 'microsoft', 'modrinth', 'services'],
    basic: {
      summary: 'Agora does not require every account for every feature. Connect only the service needed for the task you want to perform.',
      outcomes: [
        'Know which account enables each feature.',
        'Avoid confusing separate GitHub sign-ins.',
        'Disable optional services without losing local instances.',
      ],
      sections: [
        {
          title: 'Match services to purposes',
          body: 'Modrinth adds live catalog and download access. GitHub governance sign-in enables voting and community participation. Microsoft sign-in enables direct in-app online launching.',
        },
        {
          title: 'Keep account roles separate',
          body: 'The GitHub account under Accounts is for governance. Microsoft is a different identity used for Minecraft ownership and online authentication during direct launch.',
          callout: {
            tone: 'note',
            title: 'Delegated launch stays simple',
            text: 'When using the official launcher, manage the Minecraft account there. Agora does not need a Microsoft sign-in for delegated launch.',
          },
        },
        {
          title: 'Disconnect or disable',
          body: 'Turn off an integration in Services to stop using it, and use its account control to sign out where available. Existing local instances remain. Some live descriptions, catalog entries, voting, direct-launch, or AI features may no longer be available.',
        },
      ],
    },
    advanced: {
      summary: 'What to check when a service is turned on but does not seem to work, and how to keep the number of connections to a minimum.',
      outcomes: [
        'Work out why a service you turned on is not working.',
        'Know what to check when a sign-in stops working.',
        'Turn on only what you actually use.',
      ],
      sections: [
        {
          title: 'When a service is on but not working',
          body: 'A service needs to be turned on in Services, and also allowed under Privacy. For Modrinth, both the Modrinth API and its download server need to be allowed. If Lockdown Mode is on, everything is blocked, whatever the other switches say.'
        },
        {
          title: 'When a sign-in stops working',
          body: 'If voting stops working, check under Accounts that you are signed in to GitHub and that the sign-in has not expired. If direct launch says it cannot sign in, check your Microsoft account there too. That is a separate problem from Java or mod issues.',
        },
        {
          title: 'Only turn on what you use',
          body: 'We suggest turning off services you do not use. If launching through the official launcher works for you, you do not need to sign in with Microsoft in Agora at all. Privacy lets you go further and block individual kinds of connection.',
          callout: {
            tone: 'tip',
            title: 'Checking in order',
            text: 'When something will not connect, check that it is turned on, that you are signed in, that Privacy allows it, and finally that your internet is working.',
          },
        },
      ],
    },
  },
  {
    id: 'privacy-offline',
    title: 'Privacy, networking, and offline use',
    shortTitle: 'Privacy & offline',
    category: 'Customize',
    description: 'See what Agora connects to, switch connections off, and get ready to play offline.',
    keywords: ['privacy', 'network', 'lockdown', 'offline', 'telemetry', 'endpoints'],
    basic: {
      summary: 'Agora does not collect or send usage data about you. Settings lists everything Agora connects to and why, and lets you switch each connection off.',
      outcomes: [
        'Understand why Agora contacts each service category.',
        'Use Lockdown Mode as a global network block.',
        'Prepare cached data before going offline.',
      ],
      sections: [
        {
          title: 'What network access is for',
          body: 'Agora connects to the internet to find and download mods from Modrinth, update the catalog and handle voting through GitHub, download Minecraft, mod loaders and Java, sign you in with Microsoft, check for Agora updates, and for optional AI features.',
        },
        {
          title: 'Use Lockdown Mode for a global block',
          body: 'Turn on Advanced mode in Settings and open Privacy. Lockdown Mode stops Agora from connecting to anything at all. If you want more control than all-or-nothing, turn individual connections on or off instead.',
          callout: {
            tone: 'warning',
            title: 'Some things need the internet',
            text: 'While a connection is blocked or you are offline, Agora cannot sign you in or download missing game files, loaders, Java, or mods.',
          },
        },
        {
          title: 'Prepare for offline play',
          steps: [
            'Update the Agora catalog while you are online. If it never downloaded (the first run had no internet), Browse and the Home banner say so: connect, then use Download Catalog or Retry. Until it has downloaded once, there is no catalog to browse offline.',
            'Launch each required instance once so game and loader files are present.',
            'Confirm the selected Java runtime is installed.',
            'Download planned mods and packs before disconnecting.',
            'Turn on Lockdown Mode and try launching, while you can still go back online to fix anything missing.',
          ],
          body: 'Each instance needs its own files, so getting one ready for offline play does not prepare the others.',
        },
      ],
    },
    advanced: {
      summary: 'Choose exactly which connections to allow, and work out which one a feature needs when it stops working.',
      outcomes: [
        'Know which connection each feature uses.',
        'Block connections without breaking the features you want.',
        'Share logs without giving away private details.',
      ],
      sections: [
        {
          title: 'Allow only what you need',
          body: 'Privacy groups connections by what they are for: the catalog, voting, finding mods, launching, Java downloads, signing in, Agora updates, and AI. Allow the ones for the features you use. Lockdown Mode overrides all of them and blocks everything.',
          bullets: [
            'Blocking the Modrinth API also stops downloads from Modrinth.',
            'Java downloads are a separate switch from Minecraft and loader downloads.',
            'Having the catalog downloaded does not mean every mod in it is downloaded too.',
          ],
        },
        {
          title: 'When a feature stops working',
          body: 'Check that the feature is turned on in Services and that its connection is allowed under Privacy. The Privacy page also shows whether Agora\'s last attempt to reach each server worked, which tells you whether the problem is your settings or your internet.'
        },
        {
          title: 'Sharing logs safely',
          body: 'Logs can include your username, folder names, server addresses, and chat messages. Lockfiles leave out your config files, but it is still worth glancing over instance names and sources before sharing. Anything you paste into an AI assistant, or let a connected AI tool read, goes to the company that runs it.',
          callout: {
            tone: 'note',
            title: 'No tracking, but still some connections',
            text: 'Agora does not send usage data about you. Features you use, such as downloading mods, signing in, updates, and voting, still need to connect to do their job.',
          },
        },
      ],
    },
  },
  {
    id: 'governance',
    title: 'Community governance and curation',
    shortTitle: 'Community governance',
    category: 'Connect',
    description: 'Understand curated entries, triage polls, reviews, resolutions, and the transparency log.',
    keywords: ['governance', 'curation', 'vote', 'triage', 'reviews', 'transparency'],
    basic: {
      summary: 'Agora\'s catalog is community curated. Governance makes inclusion, review, and removal activity visible instead of hiding those decisions behind a private service.',
      outcomes: [
        'Read active polls and completed resolutions.',
        'Vote or flag responsibly through GitHub.',
        'Understand what the Curated label does and does not promise.',
      ],
      sections: [
        {
          title: 'Read the governance page',
          body: 'Active Triage Polls show projects currently under review. Recent Resolutions records completed outcomes. The Transparency Log lists governance actions and timestamps so catalog decisions remain inspectable.',
        },
        {
          title: 'Participate with GitHub',
          body: 'Connect GitHub under Accounts to see live poll data and follow vote links. Read the linked evidence and community rules before voting. Flag a review only for a specific policy or safety concern, not because you dislike a project.',
        },
        {
          title: 'Interpret curation',
          body: 'Curated means an entry passed Agora\'s community process and includes reviewed metadata. It does not guarantee that every version works with every other mod or that a project can never develop a security issue.',
          callout: {
            tone: 'tip',
            title: 'Check the Agora tab',
            text: 'A curated detail page can include curator notes, categories, community reviews, status, and an immunity explanation.',
          },
        },
      ],
    },
    advanced: {
      summary: 'Making sense of votes, reviews, and decisions, and how to raise a concern so it gets taken seriously.',
      outcomes: [
        'Read votes and decisions in context.',
        'Know what immunity and triage do and do not mean.',
        'Report a concern with the details curators need.',
      ],
      sections: [
        {
          title: 'Each signal means something different',
          body: 'Score, how fast votes are coming in, reviews, triage, immunity, and compatibility each tell you something different. A high score means the community likes a project, not that it will work with your mods. A mod that clashes with another is a bug, not a rule being broken.',
        },
        {
          title: 'Read decisions in context',
          body: 'A decision\'s badge tells you what kind of action was taken, and the Transparency Log says when and why. Follow the link to the discussion for the full story. A decision about one release does not always apply to every version of the project.',
        },
        {
          title: 'Raising a concern',
          body: 'Curators can act much faster when you include the exact project and version, what happens and how to make it happen, logs if they help, links to the source, and which rule you think it breaks. Please leave out personal details and stick to what you have seen.',
          callout: {
            tone: 'warning',
            title: 'Governance is not a support shortcut',
            text: 'Use project support or Crash Doctor for ordinary compatibility bugs. Use governance for curation, conduct, integrity, or community-policy concerns.',
          },
        },
      ],
    },
  },
  {
    id: 'version-migration',
    title: 'Moving an instance to a new Minecraft version',
    shortTitle: 'Version migration',
    category: 'Manage',
    description: 'Check whether every mod has a build for another Minecraft version, then move across in one confirmed step.',
    keywords: ['migration', 'minecraft version', 'upgrade', 'move version', 'abandoned mod', 'successor'],
    basic: {
      summary: 'Moving to a new Minecraft version usually fails on one or two mods rather than all of them. Agora checks first, tells you which, and only moves when you say so.',
      outcomes: [
        'Check an instance against a target version.',
        'Read the verdict and per-mod statuses.',
        'Move across, with a snapshot taken first.',
      ],
      sections: [
        {
          title: 'Check before you commit',
          body: 'Open an instance, choose a target version from the list, and check. Checking changes nothing at all — it reports whether every installed mod has a build for that version. The list only offers versions your loader actually supports, so a version you cannot reach is never presented as an option.',
        },
        {
          title: 'Read the statuses honestly',
          body: 'Each mod lands in one of a few groups, and the difference between them matters more than the total.',
          bullets: [
            'Ready: a build exists for the target version.',
            'No build yet: nothing published so far. Waiting is often enough.',
            'Looks abandoned: no activity for a long time.',
            'Has a replacement: the author points at a successor.',
            'Could not check: the lookup failed. This is not the same as bad news.',
            'Needs a look: no online identity to check against, such as a hand-added JAR.',
          ],
          callout: {
            tone: 'note',
            title: 'One unknown makes the whole report provisional',
            text: 'A mod Agora could not check is reported as unchecked, never as dead, and its presence stops the report claiming to be complete.',
          },
        },
        {
          title: 'Make the move',
          body: 'When you move, Agora first prepares everything it needs (the loader, the replacement mods) without touching the instance, then takes a snapshot, swaps every mod it can for its build on the target version, and updates the instance. If something fails while preparing, the instance was never changed and the message says so. If it fails after the swap begins, Agora undoes the changes and restores the instance to how it was; only if that undo itself fails is the snapshot named so you can restore it by hand.',
        },
      ],
    },
    advanced: {
      summary: 'What can stop a move, what to do about mods that are not ready yet, and what you will want to check yourself afterwards.',
      outcomes: [
        'Know which conditions block a migration outright.',
        'Decide what to do about mods with no target build.',
        'Understand what migration deliberately leaves alone.',
      ],
      sections: [
        {
          title: 'Locking is the only hard gate',
          body: 'A locked instance cannot be migrated; unlock it first. Everything else is a warning you can accept, including moving an instance that came from a modpack.',
          callout: {
            tone: 'warning',
            title: 'Migrating a modpack deviates from the pack',
            text: 'Most pack authors do not support running their pack on a different Minecraft version, and doing so can be unstable. A later pack update may overwrite mod versions, configuration, and other changes. Agora warns rather than refuses, because it is your instance.',
          },
        },
        {
          title: 'Mods with no target build',
          body: 'Migration does not silently drop them. You either leave them behind at their current version, accepting that they will not load, or you stop and wait for builds to appear. Either way it is an explicit answer, not a default.',
        },
        {
          title: 'What migration does not do',
          body: 'It moves mod versions. It does not migrate your worlds, rewrite configuration files for a new format, or guarantee that a mod behaves the same on the new version. Back up saves you care about before moving, and expect to revisit configuration afterwards.',
          callout: {
            tone: 'tip',
            title: 'If something breaks after the move',
            text: 'The pre-migration snapshot is a restore point, and Crash Doctor\'s guided bisect will narrow down which mod is responsible if the game starts crashing.',
          },
        },
      ],
    },
  },
  {
    id: 'disk-and-desktop',
    title: 'Disk space, shortcuts, and portable installs',
    shortTitle: 'Disk & desktop',
    category: 'Customize',
    description: 'Reclaim space, put an instance on your desktop, share screenshots, and run Agora from a USB stick.',
    keywords: ['disk space', 'reclaim', 'prune', 'shortcut', 'portable', 'screenshots', 'wrapper', 'launch history', 'storage'],
    basic: {
      summary: 'Several small features deal with where things live on disk and how you reach them. None of them are needed to play, and all of them are reversible.',
      outcomes: [
        'Reclaim runtime files no instance is using.',
        'Create a desktop shortcut for an instance.',
        'Share one screenshots folder across instances.',
      ],
      sections: [
        {
          title: 'Reclaim disk space',
          body: 'Minecraft versions, libraries, native files, and asset objects accumulate as you create instances and move between game versions. The reclaim scan lists what no instance can currently reach, grouped by category, and changes nothing until you choose categories and confirm.',
          callout: {
            tone: 'note',
            title: 'If the scan finds nothing',
            text: 'If Agora cannot read one of your instances, it will not offer to remove anything, in case that instance needs it. Fix or remove the instance it could not read, then scan again.',
          },
        },
        {
          title: 'Put an instance on your desktop',
          body: 'Instance integration can create a desktop shortcut. Opening it brings up Agora on that instance, ready for you to press Play. If Agora is already open, it uses that window rather than opening a second one.',
        },
        {
          title: 'Share a screenshots folder',
          body: 'Minecraft always writes to a screenshots folder inside the instance, with no setting to change it. Agora can link each instance to one shared folder so every screenshot lands in the same place. Existing screenshots are moved into the share rather than left behind, and unlinking stops sharing without deleting anything.',
          callout: {
            tone: 'warning',
            title: 'Linking refuses on a name collision',
            text: 'If two instances hold screenshots with the same filename, Agora stops rather than overwriting one. Rename or move the duplicate, then link again.',
          },
        },
      ],
    },
    advanced: {
      summary: 'Run Agora from a USB stick, start the game through another tool such as a frame-rate overlay, and look back at how recent launches went.',
      outcomes: [
        'Run Agora from removable media.',
        'Wrap the launch command with a tool such as a frame-rate overlay.',
        'Read launch history without over-reading it.',
      ],
      sections: [
        {
          title: 'Portable installs',
          body: 'Put an empty file named portable.txt next to Agora\'s program file, and Agora keeps all its data in a folder beside itself instead of in your user folder. To use a different folder, write its name on the first line of portable.txt. Your Microsoft and GitHub sign-ins are kept in that folder too, so anyone who can open the folder can use them. Keep the USB stick somewhere safe.',
          callout: {
            tone: 'note',
            title: 'For advanced users',
            text: 'If the AGORA_DATA_DIR environment variable is set, Agora uses that folder instead of the one in portable.txt.',
          },
        },
        {
          title: 'Wrapper commands',
          body: 'Some tools, such as frame-rate overlays, need to start the game themselves. A wrapper command tells Agora to start Minecraft through that tool. The launch preview shows the full command that will run, and the settings field tells you straight away if something is typed wrong.',
        },
        {
          title: 'Launch history',
          body: 'For each launch, Agora notes how long it took to get ready, how long you played, how it ended, and how many mods were on. Only the most recent launches are kept, nothing is sent anywhere, and the history is deleted along with the instance.',
          callout: {
            tone: 'note',
            title: 'Why there is no startup time',
            text: 'Minecraft does not report when it has finished loading, so Agora does not guess. Once there are enough launches, the history shows how recent ones compare with earlier ones.',
          },
        },
      ],
    },
  },
  {
    id: 'controller-handheld',
    title: 'Playing with a controller',
    shortTitle: 'Controller support',
    category: 'Play',
    description: 'Use Agora from the couch with a gamepad, and add controller support to Minecraft itself.',
    keywords: ['controller', 'gamepad', 'handheld', 'big picture', 'couch', 'Steam Deck', 'Controlify', 'joystick'],
    basic: {
      summary: 'Press a button on a connected controller and Agora becomes controller-driven throughout: the same app, but sized for reading at a distance and navigable with the controller. There is nothing to turn on first. Minecraft itself still needs a mod for controller input, and Agora offers to install it when you launch.',
      outcomes: [
        'Drive Agora with a gamepad, without a keyboard or mouse.',
        'Know every button and what it does.',
        'Understand why Agora offers to add Controlify.',
      ],
      sections: [
        {
          title: 'Pick the controller up',
          body: 'Connect a controller and press any button on it while Agora is open. Text and buttons get bigger, things spread out a little, and a highlight shows what is selected. These are the same pages you already know, just easier to use from the couch. There is nothing to turn on first.',
          callout: {
            tone: 'note',
            title: 'Press a button to get started',
            text: 'Agora only notices a controller once you press something on it, so one left plugged in for another game will not change anything.',
          },
        },
        {
          title: 'Move around',
          body: 'Pressing a direction moves the highlight to whatever is in that direction on screen. When you reach the edge of a list, the page scrolls to show more. The right stick scrolls the panel you are in without moving the highlight.',
          bullets: [
            'D-pad or left stick: move the highlight.',
            'A: activate what is highlighted.',
            'B: go back.',
            'LB / RB: move between sections.',
            'Start: open the command palette to jump anywhere.',
            'Right stick: scroll.',
          ],
        },
        {
          title: 'Dropdowns, sliders and typing',
          body: 'A dropdown changes in place when you press up or down on it rather than opening a system menu a controller cannot get inside, and a slider moves with left and right. Selecting a text box raises an on-screen keyboard you can drive with the stick; B closes it and puts you back on the field.',
          callout: {
            tone: 'tip',
            title: 'A keyboard and mouse never stop working',
            text: 'Nothing here takes anything away. Put the pad down and the interface goes back to its normal size, and a physical keyboard keeps working even while the on-screen one is open.',
          },
        },
        {
          title: 'Add controller support to Minecraft',
          body: 'Minecraft has no built-in gamepad support, so a controller that drives Agora will do nothing in the game itself. When you launch with a controller in use, Agora offers to install Controlify, the mod that adds it. Accepting opens the normal install flow, so you see and confirm exactly what is being added before anything changes.',
          callout: {
            tone: 'note',
            title: 'Declining is remembered',
            text: 'Choose Not now and Agora stops asking for that instance. The game launches either way. If you change your mind, install Controlify from Browse.',
          },
        },
      ],
    },
    advanced: {
      summary: 'What a controller cannot do in Agora yet, and when Agora will or will not offer to add Controlify.',
      outcomes: [
        'Know where a keyboard or mouse is still handier.',
        'Know when the Controlify offer appears.',
      ],
      sections: [
        {
          title: 'Where you may still want a keyboard',
          body: 'The on-screen keyboard covers basic letters and numbers, which is plenty for a search or a folder name. For accented letters or other languages, a physical keyboard works better. Color pickers and file pickers open windows a controller cannot reach. The living background can only be used with a mouse and keyboard.',
        },
        {
          title: 'Launching works the same',
          body: 'Pressing Play with a controller does exactly what pressing it with a mouse does, including the health check before launch.',
        },
        {
          title: 'When the Controlify offer appears',
          body: 'Agora only offers Controlify when it would be useful. It will not ask if:',
          bullets: [
            'Controlify is already installed, even if you switched it off. Installing it again would not switch it back on.',
            'The instance is locked, since a locked instance cannot have mods added.',
            'You chose Not now for that instance before.',
            'The instance uses a loader Controlify does not support. Fabric, Quilt, and NeoForge are supported.',
          ],
          callout: {
            tone: 'note',
            title: 'Added it yourself? That counts',
            text: 'If you added Controlify to the instance by hand, Agora still recognizes it and will not ask.',
          },
        },
      ],
    },
  },
  {
    id: 'crash-report-sharing',
    title: 'Getting help with a crash',
    shortTitle: 'Getting help',
    category: 'Connect',
    description: 'Export a redacted crash report to share with the community or an AI assistant of your choosing.',
    keywords: ['crash', 'report', 'export', 'share', 'discord', 'ai', 'help'],
    basic: {
      summary: 'Agora diagnoses crashes locally in Crash Doctor. When that is not enough, Crash Doctor can copy a single redacted report containing everything it found, which you can paste wherever you want help. Agora has no built-in chat assistant and never sends your crash data anywhere on its own.',
      outcomes: [
        'Export a complete crash report from Crash Doctor.',
        'Know what the report contains and what is removed.',
        'Choose a good place to ask for help.',
      ],
      sections: [
        {
          title: 'Export the report',
          body: 'Open Crash Doctor for the affected instance and choose Copy crash report. The report is assembled on your machine and placed on your clipboard. It contains the crash log, any matched crash signatures, the ranked suspect mods, and the instance\'s installed mod list — the same evidence Agora used for its own diagnosis.',
        },
        {
          title: 'Know what is shared',
          body: 'File paths and usernames are removed before the report is built, because a Minecraft crash log normally contains your home directory and therefore your real name. Nothing else is filtered, so read the report before posting it if the instance involves private server addresses or anything else you would rather not publish.',
          callout: {
            tone: 'warning',
            title: 'You control where it goes',
            text: 'Agora only puts the report on your clipboard. Pasting it into a chat service or AI assistant sends it to that provider under their terms, not Agora\'s.',
          },
        },
        {
          title: 'Choose where to ask',
          body: 'The Agora Discord reaches people who know this launcher and the curated catalog, and is the best place for questions about Agora itself. A general AI assistant is often faster for reading a stack trace. Either way, the exported report already contains the context that makes an answer possible.',
          steps: [
            'Reproduce the crash so Crash Doctor has fresh evidence.',
            'Copy the crash report.',
            'Paste it, unedited, into Discord or an assistant.',
            'Apply one reversible change at a time.',
            'Verify in Agora and Minecraft before trying the next.',
          ],
        },
      ],
    },
    advanced: {
      summary: 'Advice from someone else, or from an AI, is a good guess to test, not a sure answer. Here is how to try it safely.',
      outcomes: [
        'Try a suggested fix in a way you can undo.',
        'Tell which change actually fixed the problem.',
        'Let an AI assistant work with Agora directly, if you want.',
      ],
      sections: [
        {
          title: 'Try suggestions one at a time',
          body: 'Before acting on a suggestion, take a snapshot. Then make one change at a time, such as turning off a mod or changing Java, and launch in between, so you know which change fixed it. Be wary of any suggestion to skip Agora\'s download checks, share your passwords, or delete your worlds.',
          callout: {
            tone: 'tip',
            title: 'Ask what should happen',
            text: 'A helpful answer tells you what you should see if it is right, such as "the crash should stop once X is off". That makes it easy to check.',
          },
        },
        {
          title: 'Let an assistant use Agora directly',
          body: 'Instead of copying and pasting, you can let an AI assistant app on your computer look at your instances and crash reports and suggest changes itself, with you approving each one. See Get help from an AI assistant.',
        },
        {
          title: 'You do not need any of this',
          body: 'Crash Doctor, the console, snapshots, and lockfiles all work on your computer without outside help. Asking others is always optional.',
        },
      ],
    },
  },
  {
    id: 'ai-assistant',
    title: 'Get help from an AI assistant',
    shortTitle: 'AI assistant',
    category: 'Connect',
    description: 'Let an AI assistant app on your computer read your crash logs and fix problems through Agora, with your approval.',
    keywords: ['ai', 'assistant', 'agent', 'codex', 'chatgpt', 'claude', 'opencode', 'antigravity', 'cli', 'skill', 'help'],
    basic: {
      summary: 'An AI assistant app on your computer can use Agora\'s command-line tool to look at your instances, read crash reports, and suggest fixes. It asks before it changes anything, and you approve each step. Crash Doctor is still the first place to look: this is for when you want help working through a problem.',
      outcomes: [
        'Pick an assistant app that suits you.',
        'Hand the assistant Agora\'s command-line tool with one copied message.',
        'Keep control of what the assistant changes.',
      ],
      sections: [
        {
          title: 'Pick an assistant app',
          body: 'You need an assistant app that can run commands on your computer. A chat window in a web browser cannot. Any of these will do; pick the one you already have an account with, or start with the first.',
          bullets: [
            'OpenCode desktop: open source, quick to set up, and has some free models. It can also use an account you already have with most AI companies.',
            'Codex in the ChatGPT app: OpenAI\'s assistant, included with a ChatGPT account. How much you can use it depends on your plan.',
            'Claude Code: Anthropic\'s assistant, in the Claude desktop app. It needs a paid Claude plan, so it suits you if you already have one.',
            'Google Antigravity: Google\'s assistant app. Sign in with a Google account.',
          ],
          callout: {
            tone: 'note',
            title: 'Plans change',
            text: 'Agora is not affiliated with any of these companies, and what each one offers for free changes often. Check the app\'s own website before you sign up.',
          },
        },
        {
          title: 'Ask for help',
          body: 'Agora comes with a command-line tool the assistant uses to work with your instances. Agora gives you a ready-made message that tells the assistant where that tool is and how to use it safely.',
          steps: [
            'In Agora, open Settings, then Services, then AI assistant, and choose Copy message.',
            'Open your assistant app and paste the message, then finish it with your problem, such as "my Survival instance crashes on launch".',
            'The assistant asks to run commands. Reading commands, such as listing instances or reading a crash report, are safe to allow.',
            'Before it changes anything, it should explain what and why. Allow the change only if it makes sense to you.',
            'When it asks you to test, launch the instance from the Agora app as usual and tell it what happened.',
          ],
          callout: {
            tone: 'warning',
            title: 'Keep the approval prompts on',
            text: 'Most assistant apps have a setting that lets them run every command without asking. Leave it off. The assistant can reach any file you can, and the prompts are how you stay in charge.',
          },
        },
        {
          title: 'What the assistant sees',
          body: 'Anything the assistant reads is sent to the company that runs it. Crash reports can contain your computer\'s username, folder names, server addresses, and chat messages. If that matters to you, use Crash Doctor\'s redacted report instead.',
        },
      ],
    },
    advanced: {
      summary: 'How the assistant learns to use Agora, what it can reach on your computer, and when the MCP route suits you better. The approval prompts in your assistant app are what keep you in control.',
      outcomes: [
        'Understand what agora skill gives an agent.',
        'Know where the trust boundary really is.',
        'Know when MCP is the better choice.',
      ],
      sections: [
        {
          title: 'What agora skill provides',
          body: 'agora skill prints a guide in the common SKILL.md format: Agora\'s vocabulary, house rules (ask before changing anything, snapshot and dry-run first, never pass --yes on the person\'s behalf), the crash workflow, and what each exit code range means. It is compiled into the binary, so it always matches the commands that binary has, and it is printed without opening any Agora data.',
          bullets: [
            'The agent reads results with --json and checks exit codes rather than parsing human tables.',
            'The CLI resolves the same data root as the desktop app, including a portable copy\'s folder. Both can run at once: whichever is changing an instance holds its lock, and the app reloads an instance the agent changed.',
            'The guide asks the agent to have you launch from the app, since agora launch uses direct launch and needs a Microsoft sign-in.',
          ],
        },
        {
          title: 'Where the trust boundary is',
          body: 'An agent with a terminal can do anything your account can, including editing instance files without Agora. Agora\'s own confirmations stop by default in a non-interactive session and tell the agent to ask you before passing --yes, but that relies on the agent following instructions. Keep the app\'s per-command approval on, lock instances you care about, and point experiments at a disposable data root with --data-dir.',
          callout: {
            tone: 'tip',
            title: 'Undo is cheap',
            text: 'Ask the agent to take a snapshot before each change. agora snapshots restore puts the mods and configuration back, though not worlds: automatic snapshots exclude saves.',
          },
        },
        {
          title: 'When to use MCP instead',
          body: 'MCP suits AI apps that cannot run commands but can connect to a local tool server. There, Agora\'s MCP approval queue is the only way in, so it does protect you. It takes more setup: a token, a config file, and the Agora app running. See the MCP and external AI tools guide.',
        },
      ],
    },
  },
  {
    id: 'mcp-automation',
    title: 'MCP and external AI tools',
    shortTitle: 'MCP & automation',
    category: 'Connect',
    description: 'Connect an external local AI client and control what it may do to instances.',
    keywords: ['mcp', 'external ai', 'automation', 'localhost', 'bearer token', 'tools'],
    basic: {
      summary: 'MCP is an advanced optional bridge that lets a compatible AI tool inspect Agora and request supported actions. Most users do not need it: copying a crash report from Crash Doctor is simpler for occasional help, and an assistant app that can run commands is easier to set up (see Get help from an AI assistant).',
      outcomes: [
        'Decide whether MCP is appropriate for your workflow.',
        'Start and stop the local server.',
        'Understand the security impact before connecting a client.',
      ],
      sections: [
        {
          title: 'Copy and paste, or MCP?',
          body: 'For an occasional question, copying a crash report from Crash Doctor is simplest. MCP suits you if you already use an AI app that supports it and want it to look at your instances and crash reports, and ask to turn mods on or off, without you copying things back and forth.',
        },
        {
          title: 'Connect an AI app',
          body: 'Turn on Advanced mode, then AI / MCP Server, in Settings. Agora gives you a ready-made configuration to paste into your AI app, including a private access token.',
          steps: [
            'Start the MCP server when you want to use it.',
            'Copy the configuration for Kilo Code, Opencode, Claude Desktop, or another app that supports MCP.',
            'Keep the access token private, like a password.',
            'Check the app can list your instances before letting it change anything.',
            'Stop the server when you are done.',
          ],
        },
        {
          title: 'Approve changes carefully',
          body: 'An AI asking to do something does not mean it is a good idea. Check which instance it wants to change and what it wants to do. We recommend locking instances you care about and taking a snapshot before allowing changes.',
          callout: {
            tone: 'warning',
            title: 'Keep the token private',
            text: 'The server only accepts connections from your own computer, but other programs running there could still reach it. If the token might have leaked, make a new one in Settings.',
          },
        },
      ],
    },
    advanced: {
      summary: 'Connection details for setting up an app by hand, what each tool lets the AI do, and what to check when it will not connect.',
      outcomes: [
        'Set up a connection by hand and replace the token.',
        'Decide which tools need your approval.',
        'Fix connection problems without opening Agora up to other computers.',
      ],
      sections: [
        {
          title: 'Secure the connection',
          body: 'Use http://127.0.0.1:39741/sse for SSE or http://127.0.0.1:39741/mcp for streamable HTTP. Never expose either endpoint through port forwarding or a public reverse proxy. Send the token only in the Authorization header. Regeneration invalidates the previous token and requires updating every client.',
        },
        {
          title: 'Scope tool approvals',
          body: 'The ten tools are list_instances, list_instance_mods, read_mod_manifest, get_system_context, search_crash_signatures, search_knowledge_base, disable_mod, enable_mod, read_latest_crash, and suggest_mod_incompatibility. Use per-instance approval settings for modifying tools. Read-only inspection is lower risk but can still expose instance names, file metadata, or crash context to the connected AI provider.',
        },
        {
          title: 'Troubleshoot methodically',
          body: 'Check that the server status is running, the URL uses 127.0.0.1 and port 39741, the client transport matches the URL (streamable HTTP for /mcp, SSE for /sse), the token is current, and local security software is not blocking the process. Do not disable system security or bind the server broadly to solve a client configuration error.',
          callout: {
            tone: 'note',
            title: 'Preserve Agora\'s safety rails',
            text: 'Prefer MCP tools that route modifications through Agora\'s existing dependency, snapshot, and health workflows rather than direct filesystem commands.',
          },
        },
      ],
    },
  },
];

export const GUIDE_CATEGORIES: GuideTopic['category'][] = [
  'Start',
  'Play',
  'Manage',
  'Recover',
  'Customize',
  'Connect',
];
