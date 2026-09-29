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





T = TypeVar("T", bound="PageTenantResponseDataItem")



@_attrs_define
class PageTenantResponseDataItem:
    """ Response for tenant API calls

        Attributes:
            id (str):
            name (str):
            status (str):
            created_at (datetime.datetime):
            updated_at (datetime.datetime):
            sip_domains (list[str]): SIP domains claimed by this tenant. See `Tenant::sip_domains`.
            platform_trunk_fallback (bool): Whether this tenant's calls may leave on the platform's trunks when
                it has none of its own that can take them (`docs/TENANT_TRUNKS.md`).
            calling (TenantCalling): A tenant's calling defaults.
            description (None | str | Unset):
            contact_email (None | str | Unset):
            contact_phone (None | str | Unset):
            max_users (int | None | Unset):
            max_phone_numbers (int | None | Unset):
            max_concurrent_calls (int | None | Unset):
            billing_id (None | str | Unset):
            trial_expires_at (datetime.datetime | None | Unset):
            metadata (None | str | Unset):
     """

    id: str
    name: str
    status: str
    created_at: datetime.datetime
    updated_at: datetime.datetime
    sip_domains: list[str]
    platform_trunk_fallback: bool
    calling: TenantCalling
    description: None | str | Unset = UNSET
    contact_email: None | str | Unset = UNSET
    contact_phone: None | str | Unset = UNSET
    max_users: int | None | Unset = UNSET
    max_phone_numbers: int | None | Unset = UNSET
    max_concurrent_calls: int | None | Unset = UNSET
    billing_id: None | str | Unset = UNSET
    trial_expires_at: datetime.datetime | None | Unset = UNSET
    metadata: None | str | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        from ..models.tenant_calling import TenantCalling # noqa: PLC0415
        id = self.id

        name = self.name

        status = self.status

        created_at = self.created_at.isoformat()

        updated_at = self.updated_at.isoformat()

        sip_domains = self.sip_domains



        platform_trunk_fallback = self.platform_trunk_fallback

        calling = self.calling.to_dict()

        description: None | str | Unset
        if isinstance(self.description, Unset):
            description = UNSET
        else:
            description = self.description

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


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "id": id,
            "name": name,
            "status": status,
            "created_at": created_at,
            "updated_at": updated_at,
            "sip_domains": sip_domains,
            "platform_trunk_fallback": platform_trunk_fallback,
            "calling": calling,
        })
        if description is not UNSET:
            field_dict["description"] = description
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

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        from ..models.tenant_calling import TenantCalling # noqa: PLC0415
        d = dict(src_dict)
        id = d.pop("id")

        name = d.pop("name")

        status = d.pop("status")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        updated_at = datetime.datetime.fromisoformat(d.pop("updated_at"))




        sip_domains = cast(list[str], d.pop("sip_domains"))


        platform_trunk_fallback = d.pop("platform_trunk_fallback")

        calling = TenantCalling.from_dict(d.pop("calling"))




        def _parse_description(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        description = _parse_description(d.pop("description", UNSET))


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


        page_tenant_response_data_item = cls(
            id=id,
            name=name,
            status=status,
            created_at=created_at,
            updated_at=updated_at,
            sip_domains=sip_domains,
            platform_trunk_fallback=platform_trunk_fallback,
            calling=calling,
            description=description,
            contact_email=contact_email,
            contact_phone=contact_phone,
            max_users=max_users,
            max_phone_numbers=max_phone_numbers,
            max_concurrent_calls=max_concurrent_calls,
            billing_id=billing_id,
            trial_expires_at=trial_expires_at,
            metadata=metadata,
        )


        page_tenant_response_data_item.additional_properties = d
        return page_tenant_response_data_item

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
