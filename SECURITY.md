# Security Policy

## Overview

HyperChain is committed to ensuring the long-term security of its heterogeneous, post-quantum Layer-0 protocol. As the project is currently in Phase 1 - Foundation (In Progress), no official releases have been published, and thus no versions are currently supported with security updates. This policy outlines our approach to security during development and provides guidance for reporting vulnerabilities as we progress toward testnet and mainnet launches.

## Supported Versions

As HyperChain is in pre-release development, no versions are currently supported with security updates. Once the initial testnet is launched, we will publish a versioning scheme and designate supported versions in this section. Security updates will be provided for all active testnet and mainnet releases, with a deprecation process outlined prior to end-of-life for any version.

## Reporting a Vulnerability

How to report a vulnerability, where to go, how often can expect to get an update on a reported vulnerability, what to expect if the vulnerability is accepted or declined, etc.

### Reporting Process
HyperChain welcomes responsible disclosure of security vulnerabilities to strengthen the protocol. To report a vulnerability, please follow these steps:

1. **Contact the Security Team**: Until a dedicated security email is established, submit reports via a GitHub issue marked as "confidential" in the [HyperChain repository](https://github.com/trvorth/hyperchain/issues/new?assignees=&labels=confidential&template=security_issue.md). Include a clear description of the issue, the affected component (e.g., `hyperdag.rs`, `consensus.rs`), steps to reproduce, potential impact, and any proposed mitigation.
2. **Provide Details**: Ensure your report includes sufficient technical detail to allow our team to assess the issue.
3. **Confidentiality**: Do not disclose the vulnerability publicly until it has been addressed. We will treat all reports with strict confidentiality.

### Response Timeline
- **Acknowledgment**: You will receive an initial confirmation within 48 hours of submission via GitHub or a private communication channel if necessary.
- **Assessment**: Our security team, in collaboration with the on-chain Intrusion Detection System (IDS) module, will evaluate the report within 7 days.
- **Updates**: We will provide weekly progress updates (every 7 days) via the same channel used for acknowledgment.
- **Resolution**: Once validated, vulnerabilities will be patched in the next development milestone, with a timeline communicated to the reporter.

### Outcome Expectations
- **Accepted Vulnerabilities**: If a report is accepted, we will prioritize a fix, integrate it into the codebase, and schedule deployment for the next testnet release. Reporters may be acknowledged (with consent) in the release notes.
- **Declined Reports**: If a vulnerability is deemed out of scope, non-exploitable, or invalid, we will provide a detailed explanation within 14 days of assessment.
- **Coordination**: For critical issues (e.g., those affecting post-quantum cryptographic integrity), we may request additional collaboration with the reporter and third-party auditors.

### Rewards Program
No formal bug bounty program is active during Phase 1. We plan to introduce a rewards system with the testnet launch, with details to be included in the Testnet Launch Plan (docs/testnet_launch_plan.md).

## Security Development Practices

### Current Status
As a pre-release project, HyperChain is actively developing its security infrastructure. Key measures include:
- Implementation of lattice-based signatures (CRYSTALS-Dilithium) for post-quantum security.
- Integration of an on-chain IDS to detect validator anomalies.
- Regular code reviews by the core development team.

### Future Audits
We are preparing for a comprehensive third-party security audit prior to mainnet launch. The audit plan, including scope and timeline, will be detailed in our Security Audit Plan (docs/security_audit_plan.md) once available.

### Community Involvement
We encourage the community to contribute to security through code reviews and vulnerability reports. Please refer to our Contribution Guidelines (CONTRIBUTING.md) for participation details. All contributors are expected to adhere to our Code of Conduct.

## Legal and Ethical Considerations

Reporting vulnerabilities to HyperChain implies agreement to act in good faith. Malicious exploitation, public disclosure without coordination, or unauthorized access attempts will be treated as violations of the MIT License and may result in legal action.

## Contact

For security-related inquiries beyond vulnerability reports, use the [HyperChain GitHub Discussions](https://github.com/trvorth/hyperchain/discussions) until a dedicated contact method is established. For general support, refer to community channels to be announced with the project website (https://hyperchain.pro, forthcoming).
