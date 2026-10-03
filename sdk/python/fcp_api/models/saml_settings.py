from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="SamlSettings")



@_attrs_define
class SamlSettings:
    """ A SAML identity provider, from its metadata.

        Attributes:
            idp_metadata_xml (str | Unset): The provider's metadata XML, as it publishes it: its entity id, its
                single sign-on and logout addresses and the certificates it signs
                with. Fetched from `idp_metadata_url` when that is set.
            idp_metadata_url (None | str | Unset): Where the provider publishes its metadata: fetched when the
                provider is saved and when it is refreshed, so a certificate the
                provider rotates is picked up.
            sp_certificate (None | str | Unset): FCP's own certificate for this provider (base64 DER), in its
                metadata: what the provider checks FCP's signed requests and
                logout answers with, and encrypts assertions to. Made by the
                server — its private key is the provider's sealed secret — and
                never taken from a request.
            trust_email (bool | Unset): Take the email attribute as the person's own, for linking by
                email. SAML has no `email_verified`; turn this on only for a
                provider whose directory owns its users' addresses.
     """

    idp_metadata_xml: str | Unset = UNSET
    idp_metadata_url: None | str | Unset = UNSET
    sp_certificate: None | str | Unset = UNSET
    trust_email: bool | Unset = UNSET





    def to_dict(self) -> dict[str, Any]:
        idp_metadata_xml = self.idp_metadata_xml

        idp_metadata_url: None | str | Unset
        if isinstance(self.idp_metadata_url, Unset):
            idp_metadata_url = UNSET
        else:
            idp_metadata_url = self.idp_metadata_url

        sp_certificate: None | str | Unset
        if isinstance(self.sp_certificate, Unset):
            sp_certificate = UNSET
        else:
            sp_certificate = self.sp_certificate

        trust_email = self.trust_email


        field_dict: dict[str, Any] = {}

        field_dict.update({
        })
        if idp_metadata_xml is not UNSET:
            field_dict["idp_metadata_xml"] = idp_metadata_xml
        if idp_metadata_url is not UNSET:
            field_dict["idp_metadata_url"] = idp_metadata_url
        if sp_certificate is not UNSET:
            field_dict["sp_certificate"] = sp_certificate
        if trust_email is not UNSET:
            field_dict["trust_email"] = trust_email

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        idp_metadata_xml = d.pop("idp_metadata_xml", UNSET)

        def _parse_idp_metadata_url(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        idp_metadata_url = _parse_idp_metadata_url(d.pop("idp_metadata_url", UNSET))


        def _parse_sp_certificate(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        sp_certificate = _parse_sp_certificate(d.pop("sp_certificate", UNSET))


        trust_email = d.pop("trust_email", UNSET)

        saml_settings = cls(
            idp_metadata_xml=idp_metadata_xml,
            idp_metadata_url=idp_metadata_url,
            sp_certificate=sp_certificate,
            trust_email=trust_email,
        )

        return saml_settings

