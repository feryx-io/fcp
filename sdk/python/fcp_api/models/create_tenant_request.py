from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime

if TYPE_CHECKING:
  from ..models.tenant_calling import TenantCalling
  from ..models.tenant_directory import TenantDirectory
  from ..models.tenant_lines import TenantLines
  from ..models.tenant_offboarding import TenantOffboarding
  from ..models.tenant_provisioning import TenantProvisioning
  from ..models.tenant_sign_in import TenantSignIn





T = TypeVar("T", bound="CreateTenantRequest")



@_attrs_define
class CreateTenantRequest:
    """ Request to create a new tenant

        Attributes:
            name (str):
            description (None | str | Unset):
            status (None | str | Unset):
            contact_email (None | str | Unset):
            contact_phone (None | str | Unset):
            max_users (int | None | Unset):
            max_phone_numbers (int | None | Unset):
            max_concurrent_calls (int | None | Unset):
            billing_id (None | str | Unset):
            trial_expires_at (datetime.datetime | None | Unset):
            metadata (None | str | Unset):
            sip_domains (list[str] | Unset): SIP domains to claim on behalf of the new tenant.
            platform_trunk_fallback (bool | None | Unset): Whether the platform's trunks may carry this tenant's calls when
                it
                has none of its own that can (`docs/TENANT_TRUNKS.md` §5). True
                unless said otherwise.
            calling (None | TenantCalling | Unset):
            login_domains (list[str] | Unset): Email domains whose people are this tenant's.
            sign_in (None | TenantSignIn | Unset):
            offboarding (None | TenantOffboarding | Unset):
            directory (None | TenantDirectory | Unset):
            lines (None | TenantLines | Unset):
            provisioning (None | TenantProvisioning | Unset):
     """

    name: str
    description: None | str | Unset = UNSET
    status: None | str | Unset = UNSET
    contact_email: None | str | Unset = UNSET
    contact_phone: None | str | Unset = UNSET
    max_users: int | None | Unset = UNSET
    max_phone_numbers: int | None | Unset = UNSET
    max_concurrent_calls: int | None | Unset = UNSET
    billing_id: None | str | Unset = UNSET
    trial_expires_at: datetime.datetime | None | Unset = UNSET
    metadata: None | str | Unset = UNSET
    sip_domains: list[str] | Unset = UNSET
    platform_trunk_fallback: bool | None | Unset = UNSET
    calling: None | TenantCalling | Unset = UNSET
    login_domains: list[str] | Unset = UNSET
    sign_in: None | TenantSignIn | Unset = UNSET
    offboarding: None | TenantOffboarding | Unset = UNSET
    directory: None | TenantDirectory | Unset = UNSET
    lines: None | TenantLines | Unset = UNSET
    provisioning: None | TenantProvisioning | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.tenant_calling import TenantCalling # noqa: PLC0415
        from ..models.tenant_directory import TenantDirectory # noqa: PLC0415
        from ..models.tenant_lines import TenantLines # noqa: PLC0415
        from ..models.tenant_offboarding import TenantOffboarding # noqa: PLC0415
        from ..models.tenant_provisioning import TenantProvisioning # noqa: PLC0415
        from ..models.tenant_sign_in import TenantSignIn # noqa: PLC0415
        name = self.name

        description: None | str | Unset
        if isinstance(self.description, Unset):
            description = UNSET
        else:
            description = self.description

        status: None | str | Unset
        if isinstance(self.status, Unset):
            status = UNSET
        else:
            status = self.status

        contact_email: None | str | Unset
        if isinstance(self.contact_email, Unset):
            contact_email = UNSET
        else:
            contact_email = self.contact_email

        contact_phone: None | str | Unset
        if isinstance(self.contact_phone, Unset):
            contact_phone = UNSET
        else:
            contact_phone = self.contact_phone

        max_users: int | None | Unset
        if isinstance(self.max_users, Unset):
            max_users = UNSET
        else:
            max_users = self.max_users

        max_phone_numbers: int | None | Unset
        if isinstance(self.max_phone_numbers, Unset):
            max_phone_numbers = UNSET
        else:
            max_phone_numbers = self.max_phone_numbers

        max_concurrent_calls: int | None | Unset
        if isinstance(self.max_concurrent_calls, Unset):
            max_concurrent_calls = UNSET
        else:
            max_concurrent_calls = self.max_concurrent_calls

        billing_id: None | str | Unset
        if isinstance(self.billing_id, Unset):
            billing_id = UNSET
        else:
            billing_id = self.billing_id

        trial_expires_at: None | str | Unset
        if isinstance(self.trial_expires_at, Unset):
            trial_expires_at = UNSET
        elif isinstance(self.trial_expires_at, datetime.datetime):
            trial_expires_at = self.trial_expires_at.isoformat()
        else:
            trial_expires_at = self.trial_expires_at

        metadata: None | str | Unset
        if isinstance(self.metadata, Unset):
            metadata = UNSET
        else:
            metadata = self.metadata

        sip_domains: list[str] | Unset = UNSET
        if not isinstance(self.sip_domains, Unset):
            sip_domains = self.sip_domains



        platform_trunk_fallback: bool | None | Unset
        if isinstance(self.platform_trunk_fallback, Unset):
            platform_trunk_fallback = UNSET
        else:
            platform_trunk_fallback = self.platform_trunk_fallback

        calling: dict[str, Any] | None | Unset
        if isinstance(self.calling, Unset):
            calling = UNSET
        elif isinstance(self.calling, TenantCalling):
            calling = self.calling.to_dict()
        else:
            calling = self.calling

        login_domains: list[str] | Unset = UNSET
        if not isinstance(self.login_domains, Unset):
            login_domains = self.login_domains



        sign_in: dict[str, Any] | None | Unset
        if isinstance(self.sign_in, Unset):
            sign_in = UNSET
        elif isinstance(self.sign_in, TenantSignIn):
            sign_in = self.sign_in.to_dict()
        else:
            sign_in = self.sign_in

        offboarding: dict[str, Any] | None | Unset
        if isinstance(self.offboarding, Unset):
            offboarding = UNSET
        elif isinstance(self.offboarding, TenantOffboarding):
            offboarding = self.offboarding.to_dict()
        else:
            offboarding = self.offboarding

        directory: dict[str, Any] | None | Unset
        if isinstance(self.directory, Unset):
            directory = UNSET
        elif isinstance(self.directory, TenantDirectory):
            directory = self.directory.to_dict()
        else:
            directory = self.directory

        lines: dict[str, Any] | None | Unset
        if isinstance(self.lines, Unset):
            lines = UNSET
        elif isinstance(self.lines, TenantLines):
            lines = self.lines.to_dict()
        else:
            lines = self.lines

        provisioning: dict[str, Any] | None | Unset
        if isinstance(self.provisioning, Unset):
            provisioning = UNSET
        elif isinstance(self.provisioning, TenantProvisioning):
            provisioning = self.provisioning.to_dict()
        else:
            provisioning = self.provisioning


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "name": name,
        })
        if description is not UNSET:
            field_dict["description"] = description
        if status is not UNSET:
            field_dict["status"] = status
        if contact_email is not UNSET:
            field_dict["contact_email"] = contact_email
        if contact_phone is not UNSET:
            field_dict["contact_phone"] = contact_phone
        if max_users is not UNSET:
            field_dict["max_users"] = max_users
        if max_phone_numbers is not UNSET:
            field_dict["max_phone_numbers"] = max_phone_numbers
        if max_concurrent_calls is not UNSET:
            field_dict["max_concurrent_calls"] = max_concurrent_calls
        if billing_id is not UNSET:
            field_dict["billing_id"] = billing_id
        if trial_expires_at is not UNSET:
            field_dict["trial_expires_at"] = trial_expires_at
        if metadata is not UNSET:
            field_dict["metadata"] = metadata
        if sip_domains is not UNSET:
            field_dict["sip_domains"] = sip_domains
        if platform_trunk_fallback is not UNSET:
            field_dict["platform_trunk_fallback"] = platform_trunk_fallback
        if calling is not UNSET:
            field_dict["calling"] = calling
        if login_domains is not UNSET:
            field_dict["login_domains"] = login_domains
        if sign_in is not UNSET:
            field_dict["sign_in"] = sign_in
        if offboarding is not UNSET:
            field_dict["offboarding"] = offboarding
        if directory is not UNSET:
            field_dict["directory"] = directory
        if lines is not UNSET:
            field_dict["lines"] = lines
        if provisioning is not UNSET:
            field_dict["provisioning"] = provisioning

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.tenant_calling import TenantCalling # noqa: PLC0415
        from ..models.tenant_directory import TenantDirectory # noqa: PLC0415
        from ..models.tenant_lines import TenantLines # noqa: PLC0415
        from ..models.tenant_offboarding import TenantOffboarding # noqa: PLC0415
        from ..models.tenant_provisioning import TenantProvisioning # noqa: PLC0415
        from ..models.tenant_sign_in import TenantSignIn # noqa: PLC0415
        d = dict(src_dict)
        name = d.pop("name")

        def _parse_description(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        description = _parse_description(d.pop("description", UNSET))


        def _parse_status(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        status = _parse_status(d.pop("status", UNSET))


        def _parse_contact_email(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        contact_email = _parse_contact_email(d.pop("contact_email", UNSET))


        def _parse_contact_phone(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        contact_phone = _parse_contact_phone(d.pop("contact_phone", UNSET))


        def _parse_max_users(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        max_users = _parse_max_users(d.pop("max_users", UNSET))


        def _parse_max_phone_numbers(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        max_phone_numbers = _parse_max_phone_numbers(d.pop("max_phone_numbers", UNSET))


        def _parse_max_concurrent_calls(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        max_concurrent_calls = _parse_max_concurrent_calls(d.pop("max_concurrent_calls", UNSET))


        def _parse_billing_id(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        billing_id = _parse_billing_id(d.pop("billing_id", UNSET))


        def _parse_trial_expires_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                trial_expires_at_type_0 = datetime.datetime.fromisoformat(data)



                return trial_expires_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        trial_expires_at = _parse_trial_expires_at(d.pop("trial_expires_at", UNSET))


        def _parse_metadata(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        metadata = _parse_metadata(d.pop("metadata", UNSET))


        sip_domains = cast(list[str], d.pop("sip_domains", UNSET))


        def _parse_platform_trunk_fallback(data: object) -> bool | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(bool | None | Unset, data)

        platform_trunk_fallback = _parse_platform_trunk_fallback(d.pop("platform_trunk_fallback", UNSET))


        def _parse_calling(data: object) -> None | TenantCalling | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                calling_type_0 = TenantCalling.from_dict(data)



                return calling_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(None | TenantCalling | Unset, data)

        calling = _parse_calling(d.pop("calling", UNSET))


        login_domains = cast(list[str], d.pop("login_domains", UNSET))


        def _parse_sign_in(data: object) -> None | TenantSignIn | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                sign_in_type_0 = TenantSignIn.from_dict(data)



                return sign_in_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(None | TenantSignIn | Unset, data)

        sign_in = _parse_sign_in(d.pop("sign_in", UNSET))


        def _parse_offboarding(data: object) -> None | TenantOffboarding | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                offboarding_type_0 = TenantOffboarding.from_dict(data)



                return offboarding_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(None | TenantOffboarding | Unset, data)

        offboarding = _parse_offboarding(d.pop("offboarding", UNSET))


        def _parse_directory(data: object) -> None | TenantDirectory | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                directory_type_0 = TenantDirectory.from_dict(data)



                return directory_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(None | TenantDirectory | Unset, data)

        directory = _parse_directory(d.pop("directory", UNSET))


        def _parse_lines(data: object) -> None | TenantLines | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                lines_type_0 = TenantLines.from_dict(data)



                return lines_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(None | TenantLines | Unset, data)

        lines = _parse_lines(d.pop("lines", UNSET))


        def _parse_provisioning(data: object) -> None | TenantProvisioning | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, dict):
                    raise TypeError()
                provisioning_type_0 = TenantProvisioning.from_dict(data)



                return provisioning_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(None | TenantProvisioning | Unset, data)

        provisioning = _parse_provisioning(d.pop("provisioning", UNSET))


        create_tenant_request = cls(
            name=name,
            description=description,
            status=status,
            contact_email=contact_email,
            contact_phone=contact_phone,
            max_users=max_users,
            max_phone_numbers=max_phone_numbers,
            max_concurrent_calls=max_concurrent_calls,
            billing_id=billing_id,
            trial_expires_at=trial_expires_at,
            metadata=metadata,
            sip_domains=sip_domains,
            platform_trunk_fallback=platform_trunk_fallback,
            calling=calling,
            login_domains=login_domains,
            sign_in=sign_in,
            offboarding=offboarding,
            directory=directory,
            lines=lines,
            provisioning=provisioning,
        )


        create_tenant_request.additional_properties = d
        return create_tenant_request

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
