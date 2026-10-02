from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast
import datetime






T = TypeVar("T", bound="PageUserIdentityResponseDataItem")



@_attrs_define
class PageUserIdentityResponseDataItem:
    """ A user's account at one of the tenant's providers.

        Attributes:
            provider_id (str):
            subject (str):
            created_at (datetime.datetime):
            email (None | str | Unset):
            last_sign_in_at (datetime.datetime | None | Unset):
     """

    provider_id: str
    subject: str
    created_at: datetime.datetime
    email: None | str | Unset = UNSET
    last_sign_in_at: datetime.datetime | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        provider_id = self.provider_id

        subject = self.subject

        created_at = self.created_at.isoformat()

        email: None | str | Unset
        if isinstance(self.email, Unset):
            email = UNSET
        else:
            email = self.email

        last_sign_in_at: None | str | Unset
        if isinstance(self.last_sign_in_at, Unset):
            last_sign_in_at = UNSET
        elif isinstance(self.last_sign_in_at, datetime.datetime):
            last_sign_in_at = self.last_sign_in_at.isoformat()
        else:
            last_sign_in_at = self.last_sign_in_at


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "provider_id": provider_id,
            "subject": subject,
            "created_at": created_at,
        })
        if email is not UNSET:
            field_dict["email"] = email
        if last_sign_in_at is not UNSET:
            field_dict["last_sign_in_at"] = last_sign_in_at

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        provider_id = d.pop("provider_id")

        subject = d.pop("subject")

        created_at = datetime.datetime.fromisoformat(d.pop("created_at"))




        def _parse_email(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        email = _parse_email(d.pop("email", UNSET))


        def _parse_last_sign_in_at(data: object) -> datetime.datetime | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            try:
                if not isinstance(data, str):
                    raise TypeError()
                last_sign_in_at_type_0 = datetime.datetime.fromisoformat(data)



                return last_sign_in_at_type_0
            except (TypeError, ValueError, AttributeError, KeyError):
                pass
            return cast(datetime.datetime | None | Unset, data)

        last_sign_in_at = _parse_last_sign_in_at(d.pop("last_sign_in_at", UNSET))


        page_user_identity_response_data_item = cls(
            provider_id=provider_id,
            subject=subject,
            created_at=created_at,
            email=email,
            last_sign_in_at=last_sign_in_at,
        )


        page_user_identity_response_data_item.additional_properties = d
        return page_user_identity_response_data_item

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
